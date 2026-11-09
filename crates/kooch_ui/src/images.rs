//! `background-image` (#1708): which asset a node shows, and decoding it into the atlas; an SVG
//! drawn at the size it shows (#1700).

use std::collections::{HashMap, HashSet};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::resource::Resources;

use kooch_ecs::ViewSize;

use crate::atlas::{TOP_CLASS, class_side, size_class};
use crate::documents::{path_of, read_bytes, stamp};
use crate::layout::CANVAS_HEIGHT;
use crate::{GlyphAtlas, UiNode, UiTree, UiTrees};

/// A node's image: the asset, the texels each edge keeps unstretched, and its tint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiImage {
    pub guid: Guid,
    /// Image texels, top-right-bottom-left. Drawn one canvas unit per texel.
    pub slice: [f32; 4],
    pub tint: [f32; 4],
}

impl UiTree {
    pub fn set_image(&mut self, node: UiNode, image: Option<UiImage>) {
        if let Some(slot) = self.image.get_mut(node.0 as usize) {
            *slot = image;
        }
    }

    pub fn image(&self, node: UiNode) -> Option<UiImage> {
        self.image.get(node.0 as usize).copied().flatten()
    }
}

/// Packs every image a tree shows that the atlas lacks, or holds at an older mtime: editing the
/// PNG shows without Play. A stat per image per frame, no read. An SVG is drawn once per size
/// class it shows at, so each node filters it down at most twice.
pub fn load_images(resources: &mut Resources) {
    let physical = resources.get::<ViewSize>().copied().unwrap_or_default().0;
    let scale = physical.y.max(1) as f32 / CANVAS_HEIGHT;
    let mut wanted = Wanted::new();
    if let Some(trees) = resources.get::<UiTrees>() {
        trees.for_each(|tree| want(tree, scale, &mut wanted));
    }
    if wanted.is_empty() {
        return;
    }
    let Some(mut atlas) = resources.remove::<GlyphAtlas>() else {
        return;
    };
    let mut server = resources.remove::<AssetServer>();
    if let Some(database) = resources.get::<AssetDatabase>() {
        pack(wanted, scale, database, server.as_mut(), &mut atlas);
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    resources.insert(atlas);
}

/// [`load_images`] for one tree drawn with an atlas of its own at `scale` pixels per canvas unit,
/// as the UI Builder's canvas is.
pub fn load_tree_images(
    tree: &UiTree,
    scale: f32,
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    atlas: &mut GlyphAtlas,
) {
    let mut wanted = Wanted::new();
    want(tree, scale, &mut wanted);
    pack(wanted, scale, database, server, atlas);
}

/// Each image wanted, and the size classes nodes show it at; 0 for a node not laid out yet.
type Wanted = HashMap<Guid, HashSet<u8>>;

/// Every image `tree` needs in the atlas, painted or cutting by its alpha.
fn want(tree: &UiTree, scale: f32, wanted: &mut Wanted) {
    let images = tree.image.iter().map(|image| image.map(|image| image.guid));
    let cuts = tree
        .mask
        .iter()
        .map(|cut| cut.image.map(|image| image.guid));
    for (index, guid) in images.enumerate().chain(cuts.enumerate()) {
        let Some(guid) = guid else {
            continue;
        };
        let shown = tree.rect(UiNode(index as u32)).map_or([0, 0], |rect| {
            // At the size its transform shows it, so a zoomed SVG is not stretched (#1880).
            let zoom = tree.zoom_of(UiNode(index as u32));
            shown_size(rect.size * zoom, scale)
        });
        wanted.entry(guid).or_default().insert(size_class(shown));
    }
}

/// A node's size in whole physical pixels, which picks its raster.
pub(crate) fn shown_size(size: glam::Vec2, scale: f32) -> [u32; 2] {
    (size * scale)
        .ceil()
        .max(glam::Vec2::ZERO)
        .as_uvec2()
        .into()
}

fn pack(
    wanted: Wanted,
    scale: f32,
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    atlas: &mut GlyphAtlas,
) {
    for (guid, classes) in wanted {
        let modified = stamp(database, guid, None, &[]);
        let path = match path_of(database, guid) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!("UI image not loaded: {error}");
                continue;
            }
        };
        let vector = kooch_core::svg::is_svg(&path);
        let classes: Vec<u8> = match vector {
            true => classes.into_iter().collect(),
            false => vec![0],
        };
        let stale: Vec<u8> = classes
            .into_iter()
            .filter(|&class| atlas.packed(guid, class) != Some(modified))
            .collect();
        if stale.is_empty() {
            continue;
        }
        let bytes = match read_bytes(server.as_deref_mut(), &path) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!("UI image not loaded: {}: {error}", path.display());
                continue;
            }
        };
        #[cfg(feature = "vello")]
        if vector {
            let svg: std::sync::Arc<[u8]> = bytes.into();
            for class in stale {
                let inserted = fitted(&svg, class, scale).map(|(size, density)| {
                    atlas.insert_vector((guid, class), modified, size, density, svg.clone())
                });
                match inserted {
                    Ok(Some(_)) => {}
                    Ok(None) => tracing::warn!("UI image {guid} does not fit the vector atlas"),
                    Err(error) => {
                        tracing::warn!("UI image not loaded: {}: {error}", path.display())
                    }
                }
            }
            continue;
        }
        for class in stale {
            let decoded = match vector {
                true => raster(&bytes, class, scale),
                false => decode(&bytes),
            };
            let inserted = decoded.map(|(rgba, size, density)| {
                atlas.insert_image((guid, class), modified, size, &rgba, density)
            });
            match inserted {
                Ok(Some(_)) => {}
                Ok(None) => tracing::warn!("UI image {guid} does not fit the atlas"),
                Err(error) => tracing::warn!("UI image not loaded: {}: {error}", path.display()),
            }
        }
    }
}

type Decoded = (Vec<u8>, (u32, u32), [f32; 2]);

fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let rgba = image::load_from_memory(bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let size = rgba.dimensions();
    Ok((rgba.into_raw(), size, [1.0; 2]))
}

/// An SVG whose longer side fills `class`; with no class yet, at its own size on screen.
fn raster(bytes: &[u8], class: u8, scale: f32) -> Result<Decoded, String> {
    let (size, _) = fitted(bytes, class, scale)?;
    let raster = kooch_core::svg::rasterize(bytes, Some(size))?;
    let density = [
        raster.width as f32 / raster.own.0,
        raster.height as f32 / raster.own.1,
    ];
    Ok((raster.rgba, (raster.width, raster.height), density))
}

/// The texels an SVG of `class` takes, and texels per unit of the file.
fn fitted(bytes: &[u8], class: u8, scale: f32) -> Result<((u32, u32), [f32; 2]), String> {
    let (w, h) = kooch_core::svg::intrinsic_size(bytes)?;
    let longer = match class {
        0 => (w.max(h) * scale).ceil(),
        class => class_side(class) as f32,
    };
    let fit = longer.min(class_side(TOP_CLASS) as f32) / w.max(h).max(f32::EPSILON);
    let size = (
        (w * fit).round().max(1.0) as u32,
        (h * fit).round().max(1.0) as u32,
    );
    let density = [
        size.0 as f32 / w.max(f32::EPSILON),
        size.1 as f32 / h.max(f32::EPSILON),
    ];
    Ok((size, density))
}
