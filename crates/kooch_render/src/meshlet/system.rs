//! Scene-builder system: ECS query → MeshInstance buffer.
//!
//! Phase 1.E.2 wiring. Bridges `MeshRenderer` components (whose
//! `mesh: Option<Guid>` field references a persistent asset GUID)
//! onto the scene-wide cull pipeline by:
//!
//! 1. Maintaining a `Guid → MeshHandle` registry. The first time an
//!    entity references a particular mesh, the caller invokes
//!    [`MeshletPipeline::register_mesh`] which adds it to the
//!    [`GlobalMeshPool`] and remembers the resulting pool index.
//! 2. Each frame, [`MeshletPipeline::collect_scene_instances`] walks
//!    `Query<&MeshRenderer, &GlobalTransform>` and emits a
//!    `Vec<MeshInstance>` ready for upload to a [`MeshletScene`].
//!
//! GUID-based addressing (this PR) replaces the slotmap-key bridging
//! that lived here previously; the new model matches Unity's GUID +
//! AssetDatabase pattern. Resolving GUID → bytes (via `AssetServer +
//! AssetDatabase + Assets<MeshletMesh>`) is the responsibility of the
//! caller — typically a startup or asset-load system that calls
//! [`MeshletPipeline::register_mesh`] once an asset is GPU-resident.
//! Wiring that caller is PR3's job; this module only owns the
//! registry + ECS walk.

use std::collections::HashMap;

use glam::Mat4;
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::lod_force_level::LodForceLevel;
use kooch_ecs::mesh_renderer::MeshRenderer;
use kooch_ecs::query::Query;

use super::asset::MeshletMesh;
use super::pool::{GlobalMeshPool, MeshHandle};
use super::scene::{LOD_FORCE_NONE, MeshInstance};

/// Owns the CPU-side state that bridges the ECS to the meshlet
/// pipeline: the global mesh pool + a registry of which assets
/// (keyed by `Guid`) have already been registered.
#[derive(Default)]
pub struct MeshletPipeline {
    pool: GlobalMeshPool,
    registry: HashMap<Guid, MeshHandle>,
}

impl MeshletPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pool(&self) -> &GlobalMeshPool {
        &self.pool
    }

    pub fn pool_mut(&mut self) -> &mut GlobalMeshPool {
        &mut self.pool
    }

    pub fn registered_count(&self) -> u32 {
        self.registry.len() as u32
    }

    /// Returns the `MeshHandle` previously assigned to `guid`, or
    /// `None` if the asset has not been registered with the pool yet.
    pub fn lookup(&self, guid: Guid) -> Option<MeshHandle> {
        self.registry.get(&guid).copied()
    }

    /// Registers `mesh` under `guid` and returns the resulting
    /// `MeshHandle`. Idempotent — repeat calls with the same `guid`
    /// return the cached pool entry without re-uploading.
    pub fn register_mesh(&mut self, guid: Guid, mesh: &MeshletMesh) -> MeshHandle {
        if let Some(cached) = self.registry.get(&guid).copied() {
            return cached;
        }
        let mesh_handle = self.pool.register(mesh);
        self.registry.insert(guid, mesh_handle);
        mesh_handle
    }

    /// Walks the ECS query and returns every distinct `Guid`
    /// referenced by a visible `MeshRenderer`. Useful as the input to
    /// "ensure all referenced meshes are GPU-resident" — duplicates
    /// collapse, order is unspecified.
    pub fn collect_referenced_guids(&self, resources: &Resources) -> Vec<Guid> {
        use std::collections::HashSet;
        let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
        let mut seen: HashSet<Guid> = HashSet::new();
        query.for_each(|(renderer, _)| {
            if !renderer.visible {
                return;
            }
            if let Some(guid) = renderer.mesh {
                seen.insert(guid);
            }
        });
        seen.into_iter().collect()
    }

    /// Walks `Query<&MeshRenderer, &GlobalTransform>` from the ECS
    /// world (`resources`) and returns the per-frame `MeshInstance`
    /// slice the scene cull dispatch should consume.
    ///
    /// Filtering rules:
    /// - `mesh` must be `Some(guid)` and the GUID must already be
    ///   registered (call [`Self::register_mesh`] before the entity
    ///   goes live; production paths can hook this off the asset-
    ///   server load callback in PR3).
    /// - `visible` must be `true`.
    /// - GUIDs not in the registry are silently dropped — emitting a
    ///   warning per skipped entity per frame would spam the log.
    ///
    /// `material_id` is resolved through the [`MaterialPipeline`]
    /// resource if present; otherwise every instance falls back to
    /// slot 0 (the white-diffuse default).
    pub fn collect_scene_instances(&self, resources: &Resources) -> Vec<MeshInstance> {
        self.collect_scene_instances_with_entities(resources).0
    }

    /// The same walk, with the entity each instance came from (#481).
    ///
    /// 🔴 Motion vectors need last frame's transform for **this object**,
    /// and the position in this vector is not an identity: the walk is an
    /// ECS query, so an entity appearing, disappearing or changing
    /// archetype renumbers everything after it. Keyed by index, a
    /// reordering would hand each instance some other object's previous
    /// matrix and produce motion vectors that are wrong without anything
    /// failing.
    ///
    /// The entity is the identity. It costs one `Vec<Entity>` per frame
    /// and it is the difference between a temporal pass that works and
    /// one that smears whenever the scene changes.
    pub fn collect_scene_instances_with_entities(
        &self,
        resources: &Resources,
    ) -> (Vec<MeshInstance>, Vec<kooch_ecs::entity::Entity>) {
        let material_pipeline = resources.get::<crate::material::MaterialPipeline>();
        // Side-channel lookup of optional LodForceLevel components.
        // The MeshRenderer query is the primary walk; per-entity we
        // do a separate point query for LodForceLevel so absence
        // costs nothing (most entities don't carry the override).
        let lod_force_lookup = collect_lod_force_levels(resources);
        let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
        let mut out = Vec::new();
        let mut entities = Vec::new();
        let mesh_descriptors = &self.pool.mesh_descriptors;
        // Per-instance prefix sum into `group_max_err`: each instance
        // reserves `mesh_descriptors[mesh_id].group_count` consecutive
        // slots starting at `running_base`. Without this, two
        // instances of the same mesh share the same slot range and
        // `atomicMax` collapses every instance's LOD to the closest
        // one's verdict (#474).
        let mut running_base: u32 = 0;
        query.for_each_entity(|entity, (renderer, transform)| {
            if !renderer.visible {
                return;
            }
            let Some(guid) = renderer.mesh else {
                return;
            };
            let Some(mesh_handle) = self.lookup(guid) else {
                return;
            };
            let material_id = match material_pipeline.as_deref() {
                Some(mp) => mp.lookup_or_fallback(renderer.material),
                None => crate::material::FALLBACK_MATERIAL_ID,
            };
            let mut instance =
                MeshInstance::new(transform.matrix, mesh_handle.mesh_id, material_id);
            if let Some(force_level) = lod_force_lookup.get(&entity).copied() {
                instance.lod_force_level = force_level as i32;
            } else {
                instance.lod_force_level = LOD_FORCE_NONE;
            }
            instance.group_base = running_base;
            // #804 — the component has carried `receive_shadows` since
            // it was written and nothing ever read it: unticking it in
            // the Inspector changed nothing at all. This is the bit that
            // makes the checkbox mean something.
            instance.flags = match renderer.receive_shadows {
                true => crate::meshlet::scene::INSTANCE_RECEIVES_SHADOWS,
                false => 0,
            };
            let group_count = mesh_descriptors
                .get(mesh_handle.mesh_id as usize)
                .map(|d| d.group_count)
                .unwrap_or(0);
            running_base = running_base.saturating_add(group_count);
            out.push(instance);
            entities.push(entity);
        });
        (out, entities)
    }

    /// Total `group_max_err` slots the scene needs given an already-
    /// collected `MeshInstance` slice. Equivalent to walking each
    /// instance and summing `mesh_descriptors[mesh_id].group_count`,
    /// but reads it from the prefix sum already stamped on each
    /// instance: `last.group_base + last.group_count`. O(1).
    pub fn instance_group_capacity(&self, instances: &[MeshInstance]) -> u32 {
        let Some(last) = instances.last() else {
            return 0;
        };
        let last_count = self
            .pool
            .mesh_descriptors
            .get(last.mesh_id as usize)
            .map(|d| d.group_count)
            .unwrap_or(0);
        last.group_base.saturating_add(last_count)
    }
}

/// Snapshot every entity that carries a [`LodForceLevel`] component
/// into a hashmap so the scene-instance collector can stamp the
/// override on the matching `MeshInstance`. Empty when no entity
/// uses the LOD inspector.
fn collect_lod_force_levels(resources: &Resources) -> HashMap<Entity, u32> {
    let mut out = HashMap::new();
    let query = Query::<&LodForceLevel>::new(resources);
    query.for_each_entity(|entity, force| {
        out.insert(entity, force.level);
    });
    out
}

/// Convenience: identity transform + a fresh material id 0. Used by
/// callers that want to spawn a default instance without building one
/// by hand.
pub fn instance_at_origin(mesh_id: u32) -> MeshInstance {
    MeshInstance::new(Mat4::IDENTITY, mesh_id, 0)
}

#[cfg(test)]
mod tests;
