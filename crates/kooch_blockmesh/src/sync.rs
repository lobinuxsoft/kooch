//! Keeping a block's render mesh and collider in step with the geometry on its entity (#1452).
//!
//! 🔴 The authoring mesh lives on the component, so this reads the **scene**, not an asset. What is
//! still addressed by `Guid` is the *upload*: `GeneratedMeshes` and `MeshRenderer` are keyed that
//! way, so each block derives a stable id from its own identity rather than naming a file.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::component::ComponentRegistry;
use kooch_physics::ColliderMeshCache;
use kooch_physics::components::SHAPE_OWN_MESH;
use kooch_render::meshlet::{GeneratedMeshes, build_default_meshlets};

use crate::Block;
use crate::BlockMesh;

mod migrate;

pub(crate) use migrate::migrate_block_sources;

/// Which blocks have already been turned into a mesh, so untouched ones are not regenerated every
/// frame.
#[derive(Debug, Default)]
pub struct BuiltBlocks {
    /// Keyed by entity, because the geometry is the entity's now: two blocks that were one asset
    /// are two meshes the moment either is edited, which is the point of #1452.
    built: HashMap<Entity, u64>,
}

impl BuiltBlocks {
    /// Marks a block as needing regeneration. What an edit calls — the tool that moved a vertex
    /// knows the mesh changed, and nothing else can tell.
    pub fn forget(&mut self, entity: Entity) {
        self.built.remove(&entity);
    }

    /// Whether this block was built from the geometry it currently has.
    pub fn is_built(&self, entity: Entity, revision: u64) -> bool {
        self.built
            .get(&entity)
            .is_some_and(|built| *built == revision)
    }
}

/// The upload id of a block's mesh: derived from the entity, never a file.
///
/// 🔴 Stable across a save and load, because it comes from the entity's own persistent identity —
/// a mesh id that changed every session would re-upload every block on every load. The index is
/// the fallback for an entity that has no identity yet, which is the frame it was spawned in.
pub fn mesh_id(resources: &Resources, entity: Entity) -> Guid {
    let stable = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<kooch_ecs::persistent_id::PersistentId>())
        .and_then(|storage| storage.get(entity))
        .map(|id| id.id.get());

    let mut bytes = [0_u8; 16];
    // A namespace of its own, so a block's mesh id cannot collide with an asset's GUID.
    bytes[..8].copy_from_slice(b"koochblk");
    bytes[8..].copy_from_slice(
        &stable
            .unwrap_or_else(|| entity.index() as u64)
            .to_le_bytes(),
    );
    Guid::from_bytes(bytes)
}

/// Generates the render mesh and collider for every block whose geometry changed, and points the
/// entity at them.
pub fn sync_blocks(resources: &mut Resources) {
    migrate::migrate_block_sources(resources);

    let blocks = blocks_of(resources);
    if blocks.is_empty() {
        return;
    }

    let unbuilt: Vec<(Entity, Guid, BlockMesh)> = {
        let built = resources.get::<BuiltBlocks>();
        blocks
            .iter()
            .filter(|(entity, _, revision, _)| {
                built.is_none_or(|built| !built.is_built(*entity, *revision))
            })
            .map(|(entity, id, _, mesh)| (*entity, *id, mesh.clone()))
            .collect()
    };

    for (entity, id, mesh) in unbuilt {
        build_one(resources, entity, id, &mesh);
    }
    point_at_meshes(resources, &blocks);
    publish_colliders(resources, &blocks);
}

/// Every block, with its upload id, the revision of its geometry and the geometry itself.
///
/// ⚠️ Hashes every block's geometry each frame. Cheap against building meshlets, and the thing to
/// measure first on a dense scene (#1450) — a written-this-frame flag on the component is the
/// obvious answer if it ever shows up.
type Found = (Entity, Guid, u64, BlockMesh);

fn blocks_of(resources: &Resources) -> Vec<Found> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(storage) = registry.get_cpu::<Block>() else {
        return Vec::new();
    };
    storage
        .iter()
        .map(|(entity, block)| {
            (
                *entity,
                mesh_id(resources, *entity),
                revision_of(&block.mesh),
                block.mesh.clone(),
            )
        })
        .collect()
}

/// A number that changes whenever the geometry does.
fn revision_of(mesh: &BlockMesh) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for point in mesh.positions() {
        for value in [point.x, point.y, point.z] {
            value.to_bits().hash(&mut hasher);
        }
    }
    // Per face, not over the flattened corners: hashing a slice takes its length in, which is
    // what tells one face of four corners from two faces of two.
    for face in mesh.faces() {
        face.hash(&mut hasher);
    }
    hasher.finish()
}

/// Builds one block's mesh and publishes it for upload.
fn build_one(resources: &mut Resources, entity: Entity, id: Guid, mesh: &BlockMesh) {
    match build_default_meshlets(&mesh.to_mesh()) {
        Ok(meshlets) => {
            let published = resources.remove::<GeneratedMeshes>().map(|mut generated| {
                generated.insert(id, meshlets);
                let waiting = generated.len();
                resources.insert(generated);
                waiting
            });
            match published {
                Some(waiting) => tracing::debug!(
                    target: "kooch_blockmesh::sync",
                    %id, faces = mesh.face_count(), waiting,
                    "built a block's mesh and published it for upload",
                ),
                // 🔴 Only a fault where something draws: the headless remote host correctly has no
                // store, and warning there is noise.
                None => match resources.get::<kooch_core::gpu::GpuContext>().is_some() {
                    true => tracing::warn!(
                        target: "kooch_blockmesh::sync",
                        %id,
                        "no GeneratedMeshes resource, so the mesh has nowhere to go",
                    ),
                    false => tracing::debug!(
                        target: "kooch_blockmesh::sync",
                        %id,
                        "nothing draws here, so the block's mesh is not uploaded",
                    ),
                },
            }
        }
        // A block mid-drag can be degenerate — zero extent, a face collapsed onto itself. That is a
        // normal frame, not a fault, so the previous upload stays and the next edit tries again.
        Err(error) => {
            tracing::debug!(
                target: "kooch_blockmesh::sync",
                %id, %error,
                "block produced no meshlets; keeping the last good one",
            );
            return;
        }
    }

    let revision = revision_of(mesh);
    if let Some(mut built) = resources.remove::<BuiltBlocks>() {
        built.built.insert(entity, revision);
        resources.insert(built);
    }
}

/// Points each block's renderer at its own mesh, and its collider at its own triangles.
fn point_at_meshes(resources: &mut Resources, blocks: &[Found]) {
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };

    if let Some(storage) = registry.get_cpu_mut::<kooch_ecs::MeshRenderer>() {
        for (entity, id, ..) in blocks {
            if let Some(renderer) = storage.get_mut(*entity) {
                renderer.mesh = Some(*id);
            }
        }
    }

    if let Some(storage) = registry.get_cpu_mut::<kooch_physics::components::Collider>() {
        for (entity, ..) in blocks {
            if let Some(collider) = storage.get_mut(*entity) {
                // 🔴 Addressed by entity, leaving `mesh` alone: a block's id in a field that means a
                // mesh file gets fed to the glTF parser (#1386).
                collider.shape = SHAPE_OWN_MESH;
            }
        }
    }
}

/// Hands each block's triangles to physics, keyed by the owning entity.
fn publish_colliders(resources: &mut Resources, blocks: &[Found]) {
    let Some(mut meshes) = resources.remove::<ColliderMeshCache>() else {
        tracing::debug!(
            target: "kooch_blockmesh::sync",
            "no ColliderMeshCache, so these blocks will not collide",
        );
        return;
    };
    for (entity, _, _, mesh) in blocks {
        meshes.insert(*entity, mesh.to_collider());
    }
    resources.insert(meshes);
}

#[cfg(test)]
mod tests;
