//! Scene-builder system: ECS query → MeshInstance buffer.

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
use super::trim::{AlphaTrim, TrimPair};

/// Owns the CPU-side state that bridges the ECS to the meshlet
/// pipeline: the global mesh pool + a registry of which assets
/// (keyed by `Guid`) have already been registered.
#[derive(Default)]
pub struct MeshletPipeline {
    pool: GlobalMeshPool,
    registry: HashMap<Guid, MeshHandle>,
    /// Which masked pairs draw a cut mesh instead of discarding per pixel (#452).
    pub trim: AlphaTrim,
}

impl MeshletPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn pool(&self) -> &GlobalMeshPool {
        &self.pool
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

    /// Registers `mesh` under `guid`, replacing whatever was there.
    pub fn replace_mesh(&mut self, guid: Guid, mesh: &MeshletMesh) -> MeshHandle {
        let mesh_handle = self.pool.register(mesh);
        self.registry.insert(guid, mesh_handle);
        mesh_handle
    }

    /// Walks the ECS query and returns every distinct `Guid` referenced by a visible
    /// `MeshRenderer`. Useful as the input to "ensure all referenced meshes are GPU-resident" —
    /// duplicates collapse, order is unspecified.
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

    /// Every visible (mesh, material) whose shader masks by uv alone (#452), each once.
    pub fn collect_trim_pairs(&self, resources: &Resources) -> Vec<TrimPair> {
        let Some(materials) = resources.get::<crate::material::MaterialPipeline>() else {
            return Vec::new();
        };
        let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
        let mut out: Vec<TrimPair> = Vec::new();
        query.for_each(|(renderer, _)| {
            let (Some(mesh), Some(material)) = (renderer.mesh, renderer.material) else {
                return;
            };
            let known = out
                .iter()
                .any(|pair| pair.mesh == mesh && pair.material == material);
            if !renderer.visible || known || self.lookup(mesh).is_none() {
                return;
            }
            let Some(slot) = materials.lookup(material) else {
                return;
            };
            // Transparent is left out on purpose: its cut region would become geometry, but its
            // shadow dithers by alpha and a trimmed caster casts solid.
            let cuttable = |surface: &crate::material::SurfaceSource| {
                surface.masked
                    && surface.still
                    && surface.kind != crate::material::ShaderKind::Transparent
            };
            if !materials
                .slot_surface(slot)
                .is_some_and(|(_, surface)| cuttable(surface))
            {
                return;
            }
            out.push(TrimPair {
                mesh,
                material,
                slot,
                stamp: materials.slot_stamp(slot),
            });
        });
        out
    }

    /// Walks `Query<&MeshRenderer, &GlobalTransform>` from the ECS world (`resources`) and returns
    /// the per-frame `MeshInstance` slice the scene cull dispatch should consume.
    pub fn collect_scene_instances(&self, resources: &Resources) -> Vec<MeshInstance> {
        self.collect_scene_instances_with_entities(resources).0
    }

    /// The same walk, with the entity each instance came from (#481).
    pub fn collect_scene_instances_with_entities(
        &self,
        resources: &Resources,
    ) -> (Vec<MeshInstance>, Vec<kooch_ecs::entity::Entity>) {
        let material_pipeline = resources.get::<crate::material::MaterialPipeline>();
        // Side-channel lookup of optional LodForceLevel components. The MeshRenderer query is the
        // primary walk; per-entity we do a separate point query for LodForceLevel so absence costs
        // nothing (most entities don't carry the override).
        let lod_force_lookup = collect_lod_force_levels(resources);
        let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
        let mut out = Vec::new();
        let mut entities = Vec::new();
        // Kept apart and appended last: a cull is handed the opaque ones alone (#452).
        let mut transparent = Vec::new();
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
            // The static cut, already in the geometry (#452). Until one is built the mesh draws as
            // it was, masked, so a pair that cannot be cut is never left undrawn.
            let cut = renderer
                .material
                .and_then(|material| self.trim.mesh_for(guid, material))
                .and_then(|trimmed| self.lookup(trimmed));
            let mesh_id = cut.unwrap_or(mesh_handle).mesh_id;
            let mut instance = MeshInstance::new(transform.matrix, mesh_id, material_id);
            if let Some(force_level) = lod_force_lookup.get(&entity).copied() {
                instance.lod_force_level = force_level as i32;
            } else {
                instance.lod_force_level = LOD_FORCE_NONE;
            }
            // it was written and nothing ever read it: unticking it in the Inspector changed
            // nothing at all. This is the bit that makes the checkbox mean something.
            instance.flags = match renderer.receive_shadows {
                true => crate::meshlet::scene::INSTANCE_RECEIVES_SHADOWS,
                false => 0,
            };
            if !renderer.cast_shadows {
                instance.flags |= crate::meshlet::scene::INSTANCE_CASTS_NO_SHADOW;
            }
            if cut.is_some() {
                instance.flags |= crate::meshlet::scene::INSTANCE_TRIMMED;
            }
            let see_through = material_pipeline
                .as_deref()
                .and_then(|mp| mp.slot_surface(material_id))
                .is_some_and(|(_, s)| s.kind == crate::material::ShaderKind::Transparent);
            if see_through {
                instance.flags |= crate::meshlet::scene::INSTANCE_TRANSPARENT;
                transparent.push((instance, entity));
            } else {
                out.push(instance);
                entities.push(entity);
            }
        });
        for (instance, entity) in transparent {
            out.push(instance);
            entities.push(entity);
        }
        // Per-instance prefix sum into `group_max_err`, after the reorder: each instance reserves
        // `mesh_descriptors[mesh_id].group_count` consecutive slots starting at `running_base`.
        let mut running_base: u32 = 0;
        for instance in &mut out {
            instance.group_base = running_base;
            let group_count = self
                .pool
                .mesh_descriptors
                .get(instance.mesh_id as usize)
                .map(|d| d.group_count)
                .unwrap_or(0);
            running_base = running_base.saturating_add(group_count);
        }
        (out, entities)
    }

    /// Total `group_max_err` slots the scene needs given an already- collected `MeshInstance`
    /// slice.
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

/// Snapshot every entity that carries a [`LodForceLevel`] component into a hashmap so the
/// scene-instance collector can stamp the override on the matching `MeshInstance`. Empty when no
/// entity uses the LOD inspector.
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
