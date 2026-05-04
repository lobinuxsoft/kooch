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
use ome_core::Guid;
use ome_core::resource::Resources;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::Query;

use super::asset::MeshletMesh;
use super::pool::{GlobalMeshPool, MeshHandle};
use super::scene::MeshInstance;

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
    pub fn collect_scene_instances(&self, resources: &Resources) -> Vec<MeshInstance> {
        let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
        let mut out = Vec::new();
        query.for_each(|(renderer, transform)| {
            if !renderer.visible {
                return;
            }
            let Some(guid) = renderer.mesh else {
                return;
            };
            let Some(mesh_handle) = self.lookup(guid) else {
                return;
            };
            out.push(MeshInstance::new(
                transform.matrix,
                mesh_handle.mesh_id,
                /* material_id */ 0,
            ));
        });
        out
    }
}

/// Convenience: identity transform + a fresh material id 0. Used by
/// callers that want to spawn a default instance without building one
/// by hand.
pub fn instance_at_origin(mesh_id: u32) -> MeshInstance {
    MeshInstance::new(Mat4::IDENTITY, mesh_id, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::{Mesh, MeshVertex};
    use crate::meshlet::build_default_meshlets;

    fn cube_mesh() -> Mesh {
        let positions = [
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
        ];
        let face_indices: [[usize; 4]; 6] = [
            [0, 1, 2, 3], [4, 5, 6, 7], [0, 1, 5, 4],
            [3, 2, 6, 7], [0, 3, 7, 4], [1, 2, 6, 5],
        ];
        let face_normal = [0.0, 1.0, 0.0];
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for corners in face_indices {
            let base = vertices.len() as u32;
            for &c in &corners {
                vertices.push(MeshVertex {
                    position: positions[c],
                    normal: face_normal,
                    uv: [0.0, 0.0],
                });
            }
            indices.extend_from_slice(&[
                base, base + 1, base + 2,
                base, base + 2, base + 3,
            ]);
        }
        Mesh::from_arrays(vertices, indices)
    }

    #[test]
    fn register_is_idempotent() {
        let mut pipeline = MeshletPipeline::new();
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let guid = Guid::new_v4();

        let h0 = pipeline.register_mesh(guid, &mesh);
        let h1 = pipeline.register_mesh(guid, &mesh);
        assert_eq!(h0, h1);
        assert_eq!(pipeline.registered_count(), 1);
    }

    #[test]
    fn distinct_guids_get_distinct_pool_handles() {
        let mut pipeline = MeshletPipeline::new();
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");

        let g1 = Guid::new_v4();
        let g2 = Guid::new_v4();
        let h1 = pipeline.register_mesh(g1, &mesh);
        let h2 = pipeline.register_mesh(g2, &mesh);
        assert_ne!(h1, h2);
        assert_eq!(pipeline.registered_count(), 2);
    }

    #[test]
    fn lookup_returns_none_before_register() {
        let pipeline = MeshletPipeline::new();
        assert!(pipeline.lookup(Guid::new_v4()).is_none());
    }

    #[test]
    fn instance_at_origin_uses_identity() {
        let inst = instance_at_origin(7);
        assert_eq!(inst.mesh_id, 7);
        let m = inst.transform_mat4();
        assert_eq!(m, Mat4::IDENTITY);
    }
}
