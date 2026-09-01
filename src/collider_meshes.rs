//! [`ColliderMeshPlugin`] — the system that fills
//! [`ColliderMeshCache`](kooch_physics::ColliderMeshCache).
//!
//! # Why it lives in the facade
//!
//! It is the one job that needs both halves: `Collider.mesh` is a GUID,
//! resolving one means the asset database, and the asset database means
//! `kooch_render`. Physics must not depend on the renderer — that would
//! tie [`PhysicsBackend`] to wgpu and make the trait unswappable — and
//! the renderer has no business knowing what a collider is. This crate
//! already sees both, and is the only one that should.
//!
//! Nothing here touches a GPU. The meshlet asset is a CPU struct; the
//! cache holds plain points. A headless host resolves collision meshes
//! exactly as a windowed game does.
//!
//! [`PhysicsBackend`]: kooch_physics::PhysicsBackend

use kooch_core::Guid;
use kooch_core::app::App;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;
use kooch_physics::components::{Collider, is_mesh_derived};
use kooch_physics::{ColliderMesh, ColliderMeshCache};
use kooch_render::meshlet::MeshletMesh;

/// Resolves the meshes mesh-derived colliders name.
pub struct ColliderMeshPlugin;

impl Plugin for ColliderMeshPlugin {
    fn build(&self, app: &mut App) {
        // `PreUpdate`, alongside the physics sync it feeds. Landing a
        // frame late is not a race: the cache's epoch is in every body's
        // spec, so a mesh arriving after the body was authored retires it
        // and the next frame rebuilds with the geometry.
        app.add_system(Stage::PreUpdate, fill_collider_meshes);
    }

    fn name(&self) -> &str {
        "ColliderMeshPlugin"
    }
}

/// Loads the mesh behind every mesh-derived collider that has no answer.
///
/// Asked once per GUID, not once per frame: an answer — including a
/// failure — is kept, so a scene where a hundred crates share one
/// collision mesh does one load.
pub fn fill_collider_meshes(resources: &mut Resources) {
    let wanted = unanswered(resources);
    if wanted.is_empty() {
        return;
    }

    for guid in wanted {
        let mesh = load_mesh(resources, guid);
        let Some(mut cache) = resources.remove::<ColliderMeshCache>() else {
            return;
        };
        match mesh {
            Some(mesh) => cache.insert(guid, mesh),
            None => cache.fail(guid),
        }
        resources.insert(cache);
    }
}

/// The GUIDs mesh-derived colliders name that the cache has no answer for.
fn unanswered(resources: &Resources) -> Vec<Guid> {
    let Some(cache) = resources.get::<ColliderMeshCache>() else {
        return Vec::new();
    };
    let Some(colliders) = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Collider>())
    else {
        return Vec::new();
    };

    let mut wanted: Vec<Guid> = colliders
        .iter()
        .filter(|(_, collider)| is_mesh_derived(collider.shape))
        .filter_map(|(_, collider)| collider.mesh)
        .filter(|guid| !cache.answered(*guid))
        .collect();
    // Component storage is a hash map, so the same scene asks in a
    // different order each run — and the cache's epoch is what bodies
    // rebuild on. Sorted and deduplicated, two runs agree.
    wanted.sort_unstable_by_key(|guid| guid.as_uuid().as_u128());
    wanted.dedup();
    wanted
}

/// The mesh behind a GUID, at full detail, or `None` when it will not
/// resolve.
///
/// Says why at `warn` — a collider that never appears is otherwise a body
/// that silently is not there, and the GUID is the only clue.
fn load_mesh(resources: &mut Resources, guid: Guid) -> Option<ColliderMesh> {
    let mut server = resources.remove::<AssetServer>()?;
    let loaded = server.load_by_guid::<MeshletMesh>(guid, resources);
    resources.insert(server);

    let handle = match loaded {
        Ok(handle) => handle,
        Err(error) => {
            tracing::warn!(
                target: "kooch::collider_mesh",
                guid = %guid,
                error = %error,
                "a collider names a mesh that will not load, so its body will not collide",
            );
            return None;
        }
    };

    let mesh = resources.get::<Assets<MeshletMesh>>()?.get(handle)?;
    let (vertices, indices) = mesh.lod0_triangles();
    if vertices.is_empty() {
        tracing::warn!(
            target: "kooch::collider_mesh",
            guid = %guid,
            "a collider names a mesh with no vertices",
        );
        return None;
    }
    Some(ColliderMesh { vertices, indices })
}

#[cfg(test)]
mod tests;
