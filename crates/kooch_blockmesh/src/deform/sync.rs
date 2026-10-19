//! Keeping a bent mesh in step with the curve it is bent along (#1446).
//!
//! The same circuit [`sync_blocks`](crate::sync_blocks) runs for a block: generate under a GUID of
//! the entity's own, publish it through [`GeneratedMeshes`], point the renderer at it. The
//! difference is only where the geometry comes from — an asset here, the component there.

use std::collections::HashMap;

use kooch_core::Guid;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::query::Query;
use kooch_ecs::spline::{Spline, curve_revision};
use kooch_render::meshlet::{GeneratedMeshes, MeshletMesh};

use super::{SplineDeform, deform, revision};

/// What each bent mesh was last built from, and the mesh itself.
///
/// 🔴 The mesh is **kept**, not only published. `GeneratedMeshes` is a drain queue: the render
/// stage empties it each frame and the only copy left is on the GPU, where picking and physics
/// cannot reach it (#1537). This is the CPU side, and it is bounded the same way the revisions
/// are — an entity that loses its curve loses its entry.
#[derive(Default)]
pub struct BentMeshes {
    built: HashMap<Entity, u64>,
    meshes: HashMap<Entity, MeshletMesh>,
}

impl BentMeshes {
    /// The mesh this entity is currently drawn with, if it has been bent yet.
    pub fn get(&self, entity: Entity) -> Option<&MeshletMesh> {
        self.meshes.get(&entity)
    }
}

/// The upload id of a bent mesh: the entity's, in a namespace of its own.
///
/// 🔴 A different namespace from a block's. One entity is free to carry both a `Block` and a
/// `SplineDeform`, and two generated meshes sharing an id is one of them silently winning.
pub fn mesh_id(resources: &Resources, entity: Entity) -> Guid {
    let stable = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<kooch_ecs::persistent_id::PersistentId>())
        .and_then(|storage| storage.get(entity))
        .map(|id| id.id.get());

    let mut bytes = [0_u8; 16];
    bytes[..8].copy_from_slice(b"koochdfm");
    bytes[8..].copy_from_slice(
        &stable
            .unwrap_or_else(|| entity.index() as u64)
            .to_le_bytes(),
    );
    Guid::from_bytes(bytes)
}

/// Re-bends every deformed mesh whose curve or settings changed.
pub fn sync_deforms(resources: &mut Resources) {
    let wanted: Vec<(Entity, u64, SplineDeform, Spline)> = {
        let mut found = Vec::new();
        Query::<(&Spline, &SplineDeform)>::new(resources).for_each_entity(|entity, (curve, of)| {
            let mixed = curve_revision(&curve.points, curve.closed) ^ revision(of);
            found.push((entity, mixed, of.clone(), curve.clone()));
        });
        found
    };
    if wanted.is_empty() {
        // Nothing to drop either: the map only grows with entities this system built.
        return;
    }

    let mut bent = resources.remove::<BentMeshes>().unwrap_or_default();
    let live: Vec<Entity> = wanted.iter().map(|(entity, ..)| *entity).collect();
    bent.built.retain(|entity, _| live.contains(entity));
    bent.meshes.retain(|entity, _| live.contains(entity));

    for (entity, mixed, of, curve) in wanted {
        let id = mesh_id(resources, entity);
        point_at(resources, entity, id);
        // Every frame like `point_at`, and for the same reason: a scene load gives the entity a
        // collider pointing at the source asset, and a bent mesh that is up to date would never
        // correct it.
        if let Some(mesh) = bent.meshes.get(&entity).cloned() {
            collide_with(resources, entity, &mesh);
        }
        if bent.built.get(&entity) == Some(&mixed) {
            continue;
        }
        // 🔴 Recorded only once the mesh is published. A bend that could not land — the asset is
        // still loading, the curve is one knot — must be tried again next frame, and recording it
        // here would skip it for ever while the author watches an entity that never bends.
        if let Some(mesh) = build(resources, entity, id, &of, &curve) {
            bent.built.insert(entity, mixed);
            bent.meshes.insert(entity, mesh);
        }
    }
    resources.insert(bent);
}

/// Bends the source and publishes it. `None` when it could not be done this frame.
fn build(
    resources: &mut Resources,
    entity: Entity,
    id: Guid,
    of: &SplineDeform,
    curve: &Spline,
) -> Option<MeshletMesh> {
    let source = of.source?;
    // The asset is still loading, which is an ordinary frame rather than a fault.
    let mesh = source_mesh(resources, source)?;
    let bent = deform(&mesh, &curve.points, curve.closed, of)?;
    // The headless remote host correctly has nowhere to draw. Not a fault, and not done.
    let mut generated = resources.remove::<GeneratedMeshes>()?;
    generated.insert(id, bent.clone());
    resources.insert(generated);
    tracing::debug!(
        target: "kooch_blockmesh::deform",
        %id, ?entity, knots = curve.points.len(),
        "bent a mesh along its curve",
    );
    Some(bent)
}

/// The loaded source asset, cloned out so the bend does not hold the store.
///
/// 🔴 The server comes **out** of `Resources` for the load, because the load needs the rest of
/// `Resources` by `&mut` to put the asset in. It goes back before any `?` — a question mark with
/// the server still in hand leaves the world without one, and nothing else would report it.
fn source_mesh(resources: &mut Resources, source: Guid) -> Option<MeshletMesh> {
    let mut server = resources.remove::<AssetServer>()?;
    let loaded = server.load_by_guid::<MeshletMesh>(source, resources);
    resources.insert(server);
    let handle = loaded.ok()?;
    resources.get::<Assets<MeshletMesh>>()?.get(handle).cloned()
}

/// Hands the bent geometry to physics, keyed by the entity that owns it.
///
/// 🔴 Only when the entity already carries a `Collider`. Adding one is an authoring decision — a
/// vine that suddenly blocks the player is a bug the author never asked for — so this fills in a
/// collider that exists and creates none.
fn collide_with(resources: &mut Resources, entity: Entity, mesh: &MeshletMesh) {
    let carries = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<kooch_physics::components::Collider>())
        .is_some_and(|storage| storage.get(entity).is_some());
    if !carries {
        return;
    }
    let Some(mut meshes) = resources.remove::<kooch_physics::ColliderMeshCache>() else {
        return;
    };
    meshes.insert(entity, crate::collider::bent_collider(mesh));
    resources.insert(meshes);

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<kooch_physics::components::Collider>()
        && let Some(collider) = storage.get_mut(entity)
    {
        // 🔴 Addressed by entity, leaving `mesh` alone: a generated id in a field that means a
        // mesh file gets fed to the glTF parser (#1386).
        collider.shape = kooch_physics::components::SHAPE_OWN_MESH;
    }
}

/// Points the entity's renderer at the generated mesh.
///
/// 🔴 Every frame, not only when it was rebuilt. A scene load gives the entity a renderer pointing
/// at the **source** asset, and a bent mesh that is up to date would never correct it — the author
/// would see the straight mesh and nothing would say why.
fn point_at(resources: &mut Resources, entity: Entity, id: Guid) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<kooch_ecs::MeshRenderer>()
        && let Some(renderer) = storage.get_mut(entity)
        && renderer.mesh != Some(id)
    {
        renderer.mesh = Some(id);
    }
}
