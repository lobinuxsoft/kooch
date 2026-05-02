//! Scene-builder system: ECS query → MeshInstance buffer.
//!
//! Phase 1.E.2 wiring. Bridges `MeshRenderer` components (with the
//! new `meshlet_mesh: Option<MeshletAssetKey>` field) onto the
//! scene-wide cull pipeline by:
//!
//! 1. Maintaining a `Handle<MeshletMesh>` → `MeshHandle` registry.
//!    The first time an entity references a particular meshlet mesh
//!    asset, [`MeshletPipeline::register_mesh`] adds it to the
//!    [`GlobalMeshPool`] and remembers the resulting pool index.
//! 2. Each frame, [`MeshletPipeline::collect_scene_instances`] walks
//!    `Query<&MeshRenderer, &GlobalTransform>` and emits a
//!    `Vec<MeshInstance>` ready for upload to a [`MeshletScene`].
//!
//! This module deliberately does *not* drive the GPU: PR-1.E.3 plumbs
//! the resulting buffer through the cull dispatcher + vbuf rasterizer.
//! Keeping the building block standalone lets the system be
//! lib-tested without a wgpu device.

use std::collections::HashMap;

use glam::Mat4;
use ome_core::assets::{Assets, Handle};
use ome_core::resource::Resources;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::Query;
use slotmap::{DefaultKey, Key, KeyData};

use super::asset::MeshletMesh;
use super::pool::{GlobalMeshPool, MeshHandle};
use super::scene::MeshInstance;

/// Owns the CPU-side state that bridges the ECS to the meshlet
/// pipeline: the global mesh pool + a registry of which assets have
/// already been registered.
#[derive(Default)]
pub struct MeshletPipeline {
    pool: GlobalMeshPool,
    /// `Handle<MeshletMesh>` → `MeshHandle` so repeat lookups don't
    /// re-register the same asset.
    registry: HashMap<Handle<MeshletMesh>, MeshHandle>,
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

    /// Returns the `MeshHandle` previously assigned to `handle`, if
    /// any. `None` means the asset has not been registered with the
    /// pool yet.
    pub fn lookup(&self, handle: Handle<MeshletMesh>) -> Option<MeshHandle> {
        self.registry.get(&handle).copied()
    }

    /// Registers `mesh` under `handle` and returns the resulting
    /// `MeshHandle`. Idempotent — repeat calls with the same
    /// `(handle, mesh)` pair return the cached pool entry.
    pub fn register_mesh(
        &mut self,
        handle: Handle<MeshletMesh>,
        mesh: &MeshletMesh,
    ) -> MeshHandle {
        if let Some(cached) = self.registry.get(&handle).copied() {
            return cached;
        }
        let mesh_handle = self.pool.register(mesh);
        self.registry.insert(handle, mesh_handle);
        mesh_handle
    }

    /// Walks `Query<&MeshRenderer, &GlobalTransform>` from the ECS
    /// world (`resources`) and returns the per-frame `MeshInstance`
    /// slice the scene cull dispatch should consume.
    ///
    /// Filtering rules:
    /// - `meshlet_mesh` must be `Some` and the resulting handle must
    ///   already be registered (call [`Self::register_mesh`] before
    ///   the entity goes live; production paths can hook this off the
    ///   asset-server load callback).
    /// - `visible` must be `true`.
    /// - The `Handle<MeshletMesh>` must still resolve in
    ///   `Assets<MeshletMesh>` — stale handles are silently dropped.
    pub fn collect_scene_instances(&self, resources: &Resources) -> Vec<MeshInstance> {
        let assets = match resources.get::<Assets<MeshletMesh>>() {
            Some(a) => a,
            None => {
                tracing::warn!(
                    target: "ome_render::meshlet::system",
                    "Assets<MeshletMesh> resource missing; emitting zero instances"
                );
                return Vec::new();
            }
        };

        let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
        let mut out = Vec::new();
        query.for_each(|(renderer, transform)| {
            if !renderer.visible {
                return;
            }
            let Some(raw_key) = renderer.meshlet_mesh else {
                return;
            };
            let handle = handle_from_key(raw_key);
            if assets.get(handle).is_none() {
                return;
            }
            let Some(mesh_handle) = self.lookup(handle) else {
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

/// Round-trip a `MeshletAssetKey` (u64 stored on `MeshRenderer`) back
/// into a typed [`Handle<MeshletMesh>`].
pub fn handle_from_key(raw: u64) -> Handle<MeshletMesh> {
    let key: DefaultKey = KeyData::from_ffi(raw).into();
    Handle::from_key(key)
}

/// Inverse of [`handle_from_key`] — extract the FFI-safe u64 from a
/// typed handle so it can be stored in the ECS component.
pub fn key_from_handle(handle: Handle<MeshletMesh>) -> u64 {
    handle.key().data().as_ffi()
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
    use ome_core::assets::Assets;

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
    fn key_round_trip_through_u64() {
        let mut assets: Assets<MeshletMesh> = Assets::new();
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let h = assets.insert(mesh);

        let raw = key_from_handle(h);
        let recovered = handle_from_key(raw);
        assert_eq!(h, recovered);
    }

    #[test]
    fn register_is_idempotent() {
        let mut pipeline = MeshletPipeline::new();
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");

        let mut assets: Assets<MeshletMesh> = Assets::new();
        let handle = assets.insert(mesh.clone());

        let h0 = pipeline.register_mesh(handle, &mesh);
        let h1 = pipeline.register_mesh(handle, &mesh);
        assert_eq!(h0, h1);
        assert_eq!(pipeline.registered_count(), 1);
    }

    #[test]
    fn lookup_returns_none_before_register() {
        let pipeline = MeshletPipeline::new();
        let mut assets: Assets<MeshletMesh> = Assets::new();
        let mesh = build_default_meshlets(&cube_mesh()).expect("build");
        let handle = assets.insert(mesh);
        assert!(pipeline.lookup(handle).is_none());
    }

    #[test]
    fn instance_at_origin_uses_identity() {
        let inst = instance_at_origin(7);
        assert_eq!(inst.mesh_id, 7);
        let m = inst.transform_mat4();
        assert_eq!(m, Mat4::IDENTITY);
    }
}
