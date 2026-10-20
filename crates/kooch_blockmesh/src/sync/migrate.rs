//! Reading a block written before its geometry lived in the scene (#1452).
//!
//! 🔴 **Ships with the move, not after it.** A block whose data went from an asset to a component
//! and did not migrate is a block that loads **empty, with nothing failing** — the silent kind of
//! data loss `code-standards` names. One pass, on load, and the `.block` file is never read again.

use kooch_core::Guid;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::component::ComponentRegistry;

use crate::Block;
use crate::BlockMesh;

/// Fills in the geometry of every block that still names a `.block`, and clears the name.
///
/// A block that carries both is a block whose geometry is already current: the asset is the stale
/// copy, so it is dropped rather than read.
pub fn migrate_block_sources(resources: &mut Resources) {
    let waiting = waiting_blocks(resources);
    if waiting.is_empty() {
        return;
    }

    for (entity, source) in waiting {
        let mesh = match load(resources, source) {
            Some(mesh) => mesh,
            // Left alone, so the next frame tries again: an asset that has not arrived yet is not
            // an asset that is missing, and clearing the source here would lose the geometry for
            // good.
            None => continue,
        };
        let faces = mesh.face_count();
        if let Some(registry) = resources.get_mut::<ComponentRegistry>()
            && let Some(mut storage) = registry.get_cpu_mut::<Block>()
            && let Some(block) = storage.get_mut(entity)
        {
            block.mesh = mesh;
            block.source = None;
        }
        tracing::info!(
            target: "kooch_blockmesh::migrate",
            %source, faces,
            "a block's geometry moved out of its asset and into the scene",
        );
    }
}

/// Blocks that still name a source and have no geometry of their own.
fn waiting_blocks(resources: &Resources) -> Vec<(Entity, Guid)> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(storage) = registry.get_cpu::<Block>() else {
        return Vec::new();
    };
    storage
        .iter()
        .filter(|(_, block)| block.mesh.face_count() == 0)
        .filter_map(|(entity, block)| block.source.map(|source| (*entity, source)))
        .collect()
}

/// The old asset, loaded once.
fn load(resources: &mut Resources, source: Guid) -> Option<BlockMesh> {
    let mut server = resources.remove::<AssetServer>()?;
    let loaded = server.load_by_guid::<BlockMesh>(source, resources);
    resources.insert(server);

    let handle = match loaded {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(
                target: "kooch_blockmesh::migrate",
                %source, %error,
                "a block names a `.block` that will not load, so its geometry is lost",
            );
            return None;
        }
    };
    resources.get::<Assets<BlockMesh>>()?.get(handle).cloned()
}
