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
            trees.for_each(|tree| wanted.extend(tree.image.iter().flatten().map(|i| i.guid)));
            wanted
        })
        .unwrap_or_default();
    if wanted.is_empty() {
        return;
    }
    let mut server = resources.remove::<AssetServer>();
    let mut decoded = Vec::new();
    if let (Some(database), Some(atlas)) = (
        resources.get::<AssetDatabase>(),
        resources.get::<GlyphAtlas>(),
    ) {
        for guid in wanted {
            let modified = stamp(database, guid, None, &[]);
            if atlas
                .image(guid)
                .is_some_and(|image| image.modified == modified)
            {
                continue;
            }
            match decode(database, server.as_mut(), guid) {
                Ok(image) => decoded.push((guid, modified, image)),
                Err(error) => tracing::warn!("UI image not loaded: {error}"),
            }
        }
    }
    if let Some(server) = server {
        resources.insert(server);
    }
    if let Some(atlas) = resources.get_mut::<GlyphAtlas>() {
        for (guid, modified, image) in decoded {
            if atlas
                .insert_image(guid, modified, image.dimensions(), image.as_raw())
                .is_none()
            {
                tracing::warn!("UI image {guid} does not fit the atlas");
            }
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
