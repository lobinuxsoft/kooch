//! `background-image` (#1708): which asset a node shows, and decoding it into the atlas; an SVG
//! drawn at the size it shows (#1700).

use std::collections::{HashMap, HashSet};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::resource::Resources;

use kooch_ecs::ViewSize;

use crate::atlas::{AtlasImage, TOP_CLASS, class_side, size_class};
use crate::crops::Crop;
use crate::documents::{path_of, read_bytes, stamp};
use crate::layout::CANVAS_HEIGHT;
use crate::{GlyphAtlas, UiNode, UiTree, UiTrees};

/// A node's image: the asset, the texels each edge keeps unstretched, and its tint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiImage {
    pub guid: Guid,
    /// Image texels, top-right-bottom-left. Drawn one canvas unit per texel. `None`: the image's
    /// own, read from its `.meta` when packed.
    pub slice: Option<[f32; 4]>,
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

impl UiImage {
    /// The slice it is drawn with: its own, or the image's as `atlas` last read it.
    pub fn slice_in(&self, atlas: &GlyphAtlas) -> [f32; 4] {
        self.slice.unwrap_or_else(|| atlas.slice(self.guid))
    }
}

/// Packs every image a tree shows that the atlas lacks, or holds at an older mtime: editing the
/// PNG shows without Play. A stat per image per frame, no read. An SVG is drawn once per size
/// class it shows at, so each node filters it down at most twice.
pub fn load_images(resources: &mut Resources) {
    let physical = resources.get::<ViewSize>().copied().unwrap_or_default().0;
    let scale = physical.y.max(1) as f32 / CANVAS_HEIGHT;
    let mut wanted = Wanted::default();
    if let (Some(trees), Some(atlas)) = (resources.get::<UiTrees>(), resources.get::<GlyphAtlas>())
    {
        trees.for_each(|tree| want(tree, scale, atlas, &mut wanted));
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
    let mut wanted = Wanted::default();
    want(tree, scale, atlas, &mut wanted);
    pack(wanted, scale, database, server, atlas);
}

/// Each image wanted, and the size classes nodes show it at; 0 for a node not laid out yet. A node
/// past the largest class wants a crop instead (#1880).
#[derive(Default)]
struct Wanted {
    classes: HashMap<Guid, HashSet<u8>>,
    crops: HashSet<(Guid, Crop)>,
}

impl Wanted {
    fn is_empty(&self) -> bool {
        self.classes.is_empty()
    }
}

/// Every image `tree` needs in the atlas, painted or cutting by its alpha.
fn want(tree: &UiTree, scale: f32, atlas: &GlyphAtlas, wanted: &mut Wanted) {
    let images = tree
        .image
        .iter()
        .map(|image| image.map(|image| (image.guid, image.slice_in(atlas))));
    let cuts = tree
        .mask
        .iter()
        .map(|cut| cut.image.map(|image| (image.guid, [1.0; 4])));
    for (index, wants) in images.enumerate().chain(cuts.enumerate()) {
        let Some((guid, slice)) = wants else {
            continue;
        };
        // A sliced image keeps its corners from the whole drawing, so it is never cropped.
        let crop = (slice == [0.0; 4])
            .then(|| crate::crops::crop_of(tree, UiNode(index as u32), scale))
            .flatten();
        if let Some(crop) = crop {
            // Listed with no class: a raster packs class 0 whatever it is asked, an SVG only the crop.
            wanted.classes.entry(guid).or_default();
            if crop.size[0] > 0 && crop.size[1] > 0 {
                wanted.crops.insert((guid, crop));
            }
            continue;
        }
        let shown = tree.rect(UiNode(index as u32)).map_or([0, 0], |rect| {
            // At the size its transform shows it, so a zoomed SVG is not stretched (#1880).
            let zoom = tree.zoom_of(UiNode(index as u32));
            shown_size(rect.size * zoom, scale)
        });
        wanted
            .classes
            .entry(guid)
            .or_default()
            .insert(size_class(shown));
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
    for (&guid, classes) in &wanted.classes {
        let fit = fit_side(classes);
        let crops: Vec<Crop> = wanted
            .crops
            .iter()
            .filter(|(of, _)| *of == guid)
            .map(|(_, crop)| *crop)
            .collect();
        let path = match path_of(database, guid) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!("UI image not loaded: {error}");
                continue;
            }
        };
        // The `.meta` counts: a slice edited in the Image view repacks with it.
        let meta = kooch_core::asset_meta::meta_path_for(&path);
        let modified = stamp(database, guid, None, std::slice::from_ref(&meta));
        let vector = kooch_core::svg::is_svg(&path);
        // A GIF plays its frames into the slot at their own size, so it is never fitted.
        let shrinks = !vector && !kooch_core::gif::is_gif(&path);
        let classes: Vec<u8> = match vector {
            true => classes.iter().copied().collect(),
            false => vec![0],
        };
        let crops: Vec<Crop> = match vector {
            true => crops
                .into_iter()
                .filter(|&crop| {
                    atlas.crop(guid, crop).map(|image| image.modified) != Some(modified)
                })
                .collect(),
            false => Vec::new(),
        };
        let stale: Vec<u8> = classes
            .into_iter()
            .filter(|&class| {
                atlas.packed(guid, class) != Some(modified)
                    || (shrinks && refit(atlas.raster(guid), fit))
            })
            .collect();
        if stale.is_empty() && crops.is_empty() {
            continue;
        }
        atlas.set_slice(guid, meta_slice(&path));
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
            for crop in crops {
                if atlas
                    .insert_crop(guid, crop, modified, svg.clone())
                    .is_none()
                {
                    tracing::warn!("UI image {guid} does not fit the vector atlas");
                }
            }
            continue;
        }
        // An animated GIF packs its first frame and plays the rest in place.
        let animation = match kooch_core::gif::is_gif(&path) {
            true => kooch_core::gif::decode(&bytes).unwrap_or_else(|error| {
                tracing::warn!("UI image not animated: {}: {error}", path.display());
                None
            }),
            false => None,
        };
        if let Some(frames) = animation {
            let first = frames.frames.first().map(Vec::as_slice).unwrap_or_default();
            if atlas
                .insert_image((guid, 0), modified, frames.size, first, [1.0; 2])
                .is_none()
            {
                tracing::warn!("UI image {guid} does not fit the atlas");
            }
            atlas.set_animation(guid, Some(std::sync::Arc::new(frames)));
            continue;
        }
        atlas.set_animation(guid, None);
        // A video packs its first frame and plays from the start in place.
        if kooch_video::is_video(&path) {
            let first =
                kooch_video::first_frame(&bytes).and_then(|first| Ok((first.rgba()?, first.size)));
            match first {
                Ok((rgba, size)) => {
                    let (rgba, size, density) = shrink(rgba, size, fit);
                    if atlas
                        .insert_image((guid, 0), modified, size, &rgba, density)
                        .is_none()
                    {
                        tracing::warn!("UI video {guid} does not fit the atlas");
                    }
                    let key = video_key(guid, modified);
                    atlas.set_video(guid, Some((key, bytes.into())));
                }
                Err(error) => tracing::warn!("UI video not loaded: {}: {error}", path.display()),
            }
            continue;
        }
        atlas.set_video(guid, None);
        for class in stale {
            let decoded = match vector {
                true => raster(&bytes, class, scale),
                false => decode(&bytes).map(|(rgba, size, _)| shrink(rgba, size, fit)),
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
    atlas.advance(wanted.classes.keys().copied());
    atlas.keep_crops(&wanted.crops);
}

/// The `[import] slice` of the image at `path`, zero when it has none.
fn meta_slice(path: &std::path::Path) -> [f32; 4] {
    let read = || -> Option<[f32; 4]> {
        let meta = kooch_core::asset_meta::read_meta(path).ok()?;
        let sides = meta.import?.get("slice")?.as_array()?.clone();
        let sides: Vec<f32> = sides
            .iter()
            .filter_map(|side| side.as_integer())
            .map(|side| side as f32)
            .collect();
        sides.try_into().ok()
    };
    read().unwrap_or([0.0; 4])
}

type Decoded = (Vec<u8>, (u32, u32), [f32; 2]);

fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let rgba = image::load_from_memory(bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let size = rgba.dimensions();
    Ok((rgba.into_raw(), size, [1.0; 2]))
}

/// The longer side a raster is packed at: the largest class it shows at, the top one before layout.
/// 🔴 Packed at the file's size, one 4K image or video frame took a 4096² atlas, 64 MB (#1893).
fn fit_side(classes: &HashSet<u8>) -> u32 {
    let shown = classes.iter().copied().max().unwrap_or(0);
    class_side(if shown == 0 { TOP_CLASS } else { shown })
}

/// Whether `packed`, a raster, is bigger than `fit` or was shrunk below it.
fn refit(packed: Option<AtlasImage>, fit: u32) -> bool {
    let Some(image) = packed else {
        return false;
    };
    let [x0, y0, x1, y1] = image.rect;
    let longer = (x1 - x0).max(y1 - y0);
    longer > fit || (image.density[0] < 1.0 && longer < fit)
}

/// `rgba` with its longer side at most `fit`, and the texels per file texel it kept.
fn shrink(rgba: Vec<u8>, (w, h): (u32, u32), fit: u32) -> Decoded {
    let longer = w.max(h);
    if longer <= fit {
        return (rgba, (w, h), [1.0; 2]);
    }
    let Some(image) = image::RgbaImage::from_raw(w, h, rgba) else {
        return (Vec::new(), (0, 0), [1.0; 2]);
    };
    let ratio = fit as f32 / longer as f32;
    let size = (
        ((w as f32 * ratio).round() as u32).max(1),
        ((h as f32 * ratio).round() as u32).max(1),
    );
    let small = image::imageops::resize(
        &image,
        size.0,
        size.1,
        image::imageops::FilterType::Triangle,
    );
    let density = [size.0 as f32 / w as f32, size.1 as f32 / h as f32];
    (small.into_raw(), size, density)
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

/// A video's name to the renderer: a file saved again plays as a new one.
fn video_key(guid: Guid, modified: std::time::SystemTime) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (guid, modified).hash(&mut hasher);
    hasher.finish()
}
