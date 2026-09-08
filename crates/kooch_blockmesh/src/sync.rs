//! Keeping a block's render mesh and collider in step with its source.

use std::collections::{HashMap, HashSet};

use kooch_core::Guid;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_physics::ColliderMeshCache;
use kooch_physics::components::SHAPE_OWN_MESH;
use kooch_render::meshlet::{GeneratedMeshes, build_default_meshlets};

use crate::Block;
use crate::BlockMesh;

/// Which sources have already been turned into a mesh.
///
/// Generating is cheap for one box and not cheap for a level, and a
/// block that nobody touched this frame is every block on most frames.
#[derive(Debug, Default)]
pub struct BuiltBlocks {
    /// The handle each built source resolved to.
    ///
    /// A handle rather than a bare "yes": picking, drawing and editing
    /// all need the mesh, they all hold `&Resources` and cannot load,
    /// and re-resolving a GUID per frame to answer the same question is
    /// the lookup this already did once.
    built: HashMap<Guid, Built>,
}

/// What a source resolved to, and the revision it was resolved at.
#[derive(Debug, Clone, Copy)]
struct Built {
    handle: kooch_core::assets::Handle<BlockMesh>,
    /// 🔴 A reload overwrites the value under the SAME handle, so the
    /// handle alone cannot say the shape changed. Without this the
    /// project built a block once and never again — its collider stayed
    /// the shape the block was born with, however far the editor moved
    /// it.
    revision: u64,
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

    /// Whether this source was built from the bytes it currently has.
    pub fn is_built(&self, guid: Guid, revision: u64) -> bool {
        self.built
            .get(&guid)
            .is_some_and(|built| built.revision == revision)
    }

    /// The handle a built source resolved to.
    ///
    /// What picking, drawing and editing all need: they hold
    /// `&Resources` and cannot load, and re-resolving a GUID per frame
    /// to answer a question this already answered is the lookup the
    /// handle exists to skip.
    pub fn handle(&self, guid: Guid) -> Option<kooch_core::assets::Handle<BlockMesh>> {
        self.built.get(&guid).map(|built| built.handle)
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
            .filter(|guid| {
                let revision = revision_of(resources, *guid);
                built.is_none_or(|built| !built.is_built(*guid, revision))
            })
            .collect()
    };

    for guid in unbuilt {
        build_one(resources, guid);
    }
    point_at_sources(resources, &sources);
    publish_colliders(resources, &sources);
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
    // Read BEFORE the load, so a write that lands mid-build is not
    // recorded as already built.
    let written = revision_of(resources, guid);
    let Some((handle, block_mesh)) = load_source(resources, guid) else {
        return;
    };

    match build_default_meshlets(&block_mesh.to_mesh()) {
        Ok(meshlets) => {
            // Said once per build, not per frame. Two processes run this
            // — the editor draws its mirror and the project owns the
            // world — and a silence that only breaks on failure cannot
            // tell you which of them built the mesh you are not seeing.
            let published = resources.remove::<GeneratedMeshes>().map(|mut generated| {
                generated.insert(guid, meshlets);
                let waiting = generated.len();
                resources.insert(generated);
                waiting
            });
            match published {
                Some(waiting) => tracing::info!(
                    target: "kooch_blockmesh::sync",
                    %guid, faces = block_mesh.face_count(), waiting,
                    "built a block's mesh and published it for upload",
                ),
                // 🔴 Only a fault where something draws. The remote
                // host simulates and renders nothing, so it has no
                // renderer and no store — a warning there is an alarm
                // about a correct absence, which is how alarms get
                // ignored. With a GPU present the store is missing, and
                // the mesh is built and dropped in silence.
                None => match resources.get::<kooch_core::gpu::GpuContext>().is_some() {
                    true => tracing::warn!(
                        target: "kooch_blockmesh::sync",
                        %guid,
                        "no GeneratedMeshes resource, so the mesh has nowhere to go",
                    ),
                    false => tracing::debug!(
                        target: "kooch_blockmesh::sync",
                        %guid,
                        "nothing draws here, so the block's mesh is not uploaded",
                    ),
                },
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

    if let Some(mut built) = resources.remove::<BuiltBlocks>() {
        built.built.insert(
            guid,
            Built {
                handle,
                revision: written,
            },
        );
        resources.insert(built);
    }
}

/// Reads a `BlockMesh` out of asset storage, loading it if needed.
type Loaded = (kooch_core::assets::Handle<BlockMesh>, BlockMesh);

fn load_source(resources: &mut Resources, guid: Guid) -> Option<Loaded> {
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
    let mesh = resources.get::<Assets<BlockMesh>>()?.get(handle).cloned()?;
    Some((handle, mesh))
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
        for (entity, _) in sources {
            if let Some(collider) = storage.get_mut(*entity) {
                // 🔴 Addressed by the entity, and `mesh` deliberately
                // left alone. Naming a `.block` in a field that means "a
                // mesh on disk" is what had two separate walks feeding
                // that file to a glTF parser.
                collider.shape = SHAPE_OWN_MESH;
            }
        }
    }
}

/// Hands each block's triangles to physics, keyed by the entity that
/// owns them.
///
/// 🔴 Per entity, not per source. Two blocks built from one `.block`
/// are two shapes the moment either is scaled, and one cache entry
/// between them is right only by luck.
fn publish_colliders(resources: &mut Resources, sources: &[(kooch_ecs::Entity, Guid)]) {
    let shapes: Vec<(kooch_ecs::Entity, kooch_physics::ColliderMesh)> = {
        let Some(assets) = resources.get::<Assets<BlockMesh>>() else {
            return;
        };
        let Some(built) = resources.get::<BuiltBlocks>() else {
            return;
        };
        sources
            .iter()
            .filter_map(|(entity, source)| {
                let mesh = assets.get(built.handle(*source)?)?;
                Some((*entity, mesh.to_collider()))
            })
            .collect()
    };

    let Some(mut meshes) = resources.remove::<ColliderMeshCache>() else {
        tracing::debug!(
            target: "kooch_blockmesh::sync",
            "no ColliderMeshCache, so these blocks will not collide",
        );
        return;
    };
    for (entity, collider) in shapes {
        meshes.insert(entity, collider);
    }
    resources.insert(meshes);
}

#[cfg(test)]
mod tests;

/// How many times this source's file has been written.
fn revision_of(resources: &Resources, guid: Guid) -> u64 {
    resources
        .get::<kooch_core::asset_loader::ReloadedAssets>()
        .map(|reloaded| reloaded.revision(guid))
        .unwrap_or_default()
}
