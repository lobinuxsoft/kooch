//! `background-image` (#1708): which asset a node shows, and decoding it into the atlas.

use std::collections::HashSet;

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;
use kooch_core::asset_loader::AssetServer;
use kooch_core::resource::Resources;

use crate::documents::{path_of, read_bytes, stamp};
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
/// PNG shows without Play. A stat per image per frame, no read.
pub fn load_images(resources: &mut Resources) {
    let wanted: HashSet<Guid> = resources
        .get::<UiTrees>()
        .map(|trees| {
            let mut wanted = HashSet::new();
            trees.for_each(|tree| wanted.extend(shown(tree)));
            wanted
        })
        .unwrap_or_default();
    if wanted.is_empty() {
        return;
    }
    let Some(mut atlas) = resources.remove::<GlyphAtlas>() else {
        return;
    };
    let mut server = resources.remove::<AssetServer>();
    if let Some(database) = resources.get::<AssetDatabase>() {
        pack(wanted, database, server.as_mut(), &mut atlas);
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    resources.insert(atlas);
}

/// [`load_images`] for one tree drawn with an atlas of its own, as the UI Builder's canvas is.
pub fn load_tree_images(
    tree: &UiTree,
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    atlas: &mut GlyphAtlas,
) {
    let wanted: HashSet<Guid> = shown(tree).collect();
    pack(wanted, database, server, atlas);
}

/// Every image `tree` needs in the atlas: painted, or cutting by its alpha.
fn shown(tree: &UiTree) -> impl Iterator<Item = Guid> + '_ {
    let images = tree.image.iter().flatten().map(|image| image.guid);
    images.chain(tree.mask.iter().flatten().copied())
}

fn pack(
    wanted: HashSet<Guid>,
    database: &AssetDatabase,
    mut server: Option<&mut AssetServer>,
    atlas: &mut GlyphAtlas,
) {
    for guid in wanted {
        let modified = stamp(database, guid, None, &[]);
        if atlas
            .image(guid)
            .is_some_and(|image| image.modified == modified)
        {
            continue;
        }
        match decode(database, server.as_deref_mut(), guid) {
            Ok(image) => {
                if atlas
                    .insert_image(guid, modified, image.dimensions(), image.as_raw())
                    .is_none()
                {
                    tracing::warn!("UI image {guid} does not fit the atlas");
                }
            }
            Err(error) => tracing::warn!("UI image not loaded: {error}"),
        }
    }
}

fn decode(
    database: &AssetDatabase,
    server: Option<&mut AssetServer>,
    guid: Guid,
) -> Result<image::RgbaImage, String> {
    let path = path_of(database, guid)?;
    let bytes = read_bytes(server, &path)?;
    image::load_from_memory(&bytes)
        .map(|image| image.to_rgba8())
        .map_err(|e| format!("{}: {e}", path.display()))
}
