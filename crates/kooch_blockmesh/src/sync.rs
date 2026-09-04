//! Keeping a block's render mesh and collider in step with its source.

use std::collections::HashSet;

use kooch_core::Guid;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_physics::ColliderMeshCache;
use kooch_physics::components::SHAPE_TRIMESH;
use kooch_render::meshlet::{GeneratedMeshes, build_default_meshlets};

use crate::Block;
use crate::BlockMesh;

/// Which sources have already been turned into a mesh.
///
/// Generating is cheap for one box and not cheap for a level, and a
/// block that nobody touched this frame is every block on most frames.
#[derive(Debug, Default)]
pub struct BuiltBlocks {
    built: HashSet<Guid>,
}

impl BuiltBlocks {
    /// Marks a source as needing regeneration. What an edit calls — the
    /// tool that moved a vertex knows the mesh changed, and nothing else
    /// can tell.
    pub fn forget(&mut self, guid: Guid) {
        self.built.remove(&guid);
    }

    /// Marks every source as needing regeneration.
    pub fn forget_all(&mut self) {
        self.built.clear();
    }

    pub fn is_built(&self, guid: Guid) -> bool {
        self.built.contains(&guid)
    }
}

/// Generates the render mesh and collider for every block whose source
/// has not been built yet, and points the entity's `MeshRenderer` and
/// `Collider` at them.
///
/// Both outputs go under the block mesh's own GUID: they are two views
/// of one shape, and giving them separate identities would let them
/// drift apart with nothing to notice.
pub fn sync_blocks(resources: &mut Resources) {
    let sources = block_sources(resources);
    if sources.is_empty() {
        return;
    }

    let unbuilt: Vec<Guid> = {
        let built = resources.get::<BuiltBlocks>();
        let mut seen = HashSet::new();
        sources
            .iter()
            .map(|(_, guid)| *guid)
            .filter(|guid| seen.insert(*guid))
            .filter(|guid| built.is_none_or(|built| !built.is_built(*guid)))
            .collect()
    };

    for guid in unbuilt {
        build_one(resources, guid);
    }
    point_at_sources(resources, &sources);
}

/// Every block that names a source, paired with it.
fn block_sources(resources: &Resources) -> Vec<(kooch_ecs::Entity, Guid)> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(storage) = registry.get_cpu::<Block>() else {
        return Vec::new();
    };
    storage
        .iter()
        .filter_map(|(entity, block)| block.source.map(|guid| (*entity, guid)))
        .collect()
}

/// Loads one source and publishes both of its outputs.
fn build_one(resources: &mut Resources, guid: Guid) {
    let Some(block_mesh) = load_source(resources, guid) else {
        return;
    };

    match build_default_meshlets(&block_mesh.to_mesh()) {
        Ok(meshlets) => {
            if let Some(mut generated) = resources.remove::<GeneratedMeshes>() {
                generated.insert(guid, meshlets);
                resources.insert(generated);
            }
        }
        // A block mid-drag can be degenerate — zero extent, a face
        // collapsed onto itself. That is a normal frame, not a fault, so
        // the previous upload stays and the next edit tries again.
        Err(error) => tracing::debug!(
            target: "kooch_blockmesh::sync",
            %guid, %error,
            "block produced no meshlets; keeping the last good one",
        ),
    }

    if let Some(mut meshes) = resources.remove::<ColliderMeshCache>() {
        meshes.insert(guid, block_mesh.to_collider());
        resources.insert(meshes);
    }

    if let Some(mut built) = resources.remove::<BuiltBlocks>() {
        built.built.insert(guid);
        resources.insert(built);
    }
}

/// Reads a `BlockMesh` out of asset storage, loading it if needed.
fn load_source(resources: &mut Resources, guid: Guid) -> Option<BlockMesh> {
    let mut server = resources.remove::<AssetServer>()?;
    let loaded = server.load_by_guid::<BlockMesh>(guid, resources);
    resources.insert(server);

    let handle = match loaded {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(
                target: "kooch_blockmesh::sync",
                %guid, %error,
                "block names a source that will not load",
            );
            return None;
        }
    };
    resources.get::<Assets<BlockMesh>>()?.get(handle).cloned()
}

/// Points each block's renderer and collider at its source's GUID.
fn point_at_sources(resources: &mut Resources, sources: &[(kooch_ecs::Entity, Guid)]) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };

    if let Some(storage) = registry.get_cpu_mut::<kooch_ecs::MeshRenderer>() {
        for (entity, guid) in sources {
            if let Some(renderer) = storage.get_mut(*entity) {
                renderer.mesh = Some(*guid);
            }
        }
    }

    if let Some(storage) = registry.get_cpu_mut::<kooch_physics::components::Collider>() {
        for (entity, guid) in sources {
            if let Some(collider) = storage.get_mut(*entity) {
                collider.shape = SHAPE_TRIMESH;
                collider.mesh = Some(*guid);
            }
        }
    }
}

#[cfg(test)]
mod tests;
