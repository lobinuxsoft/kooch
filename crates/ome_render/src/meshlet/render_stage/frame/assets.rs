//! GPU mesh cache upkeep + ECS asset sync for [`MeshletRenderStage`].
//!
//! - [`Self::ensure_gpu_mesh`] registers a single [`MeshletMesh`] under
//!   a GUID into the pool (idempotent).
//! - [`Self::gpu_mesh_count`] reports the current registry size.
//! - [`Self::sync_assets_to_gpu`] resolves every visible
//!   `MeshRenderer.mesh` GUID through the [`AssetServer`] and uploads
//!   any that aren't yet GPU-resident. Also runs the per-frame material
//!   pool sync prior to cull dispatch.

use ome_core::Guid;
use ome_core::asset_loader::AssetServer;
use ome_core::assets::Assets;
use ome_core::resource::Resources;

use crate::meshlet::asset::MeshletMesh;

use super::super::MeshletRenderStage;

impl MeshletRenderStage {
    /// Registers `mesh` under `guid` in the global mesh pool.
    /// Idempotent — the underlying [`MeshletPipeline::register_mesh`]
    /// caches by GUID, so a repeat call with the same GUID is a
    /// no-op. Marks the pool dirty so the next
    /// [`Self::render_with_assets`] rebuilds the GPU mirror.
    ///
    /// The `device` argument is kept on the signature for backward
    /// compatibility — the actual upload is deferred to render time
    /// because batched registrations only need one GPU upload.
    pub fn ensure_gpu_mesh(
        &mut self,
        _device: &wgpu::Device,
        guid: Guid,
        mesh: &MeshletMesh,
    ) {
        let before = self.pipeline.registered_count();
        let bytes_before = self.pipeline.pool().byte_size();
        self.pipeline.register_mesh(guid, mesh);
        let after = self.pipeline.registered_count();
        if after > before {
            self.pool_dirty = true;
            // #463.5 — credit the freshly-appended pool bytes to the
            // engine VRAM tracker (when wired). Idempotent calls to
            // register_mesh with the same GUID return the cached
            // handle without growing the pool, so `bytes_after ==
            // bytes_before` and the diff is zero.
            if let Some(tracker) = &self.vram_tracker {
                let bytes_after = self.pipeline.pool().byte_size();
                tracker.add(bytes_after.saturating_sub(bytes_before));
            }
        }
    }

    /// Number of distinct meshlet meshes currently registered in the
    /// pool. The GPU mirror tracks the same count once it has been
    /// rebuilt at the next render call.
    pub fn gpu_mesh_count(&self) -> u32 {
        self.pipeline.registered_count()
    }

    /// Resolves every visible `MeshRenderer.mesh` GUID through the
    /// `AssetServer`, fetches the meshlet asset from
    /// `Assets<MeshletMesh>`, and uploads any GUID that is not yet
    /// GPU-resident.
    ///
    /// Idempotent: GUIDs already in the pool's registry are skipped
    /// without touching the AssetServer or Assets storage. Per-frame
    /// cost when steady-state is one ECS query + N registry lookups.
    ///
    /// Failure modes (logged, never panic):
    /// - `AssetServer` resource missing → noop, log warn.
    /// - GUID not registered in `AssetDatabase` → log warn, skip entity.
    /// - Loader rejects the bytes → log warn, skip entity.
    /// - `Assets<MeshletMesh>` missing or stale handle → log warn, skip.
    pub fn sync_assets_to_gpu(&mut self, device: &wgpu::Device, resources: &mut Resources) {
        // Material pool sync first: the meshlet scene system reads
        // `MaterialPipeline.lookup_or_fallback` when assembling
        // `MeshInstance.material_id`, so any newly-picked GUID has
        // to be in the registry before the cull dispatch fires.
        // Pull the GPU queue out of GpuContext so we can drive the
        // sync without expanding this method's signature.
        if let Some(mut material_pipeline) =
            resources.remove::<crate::material::MaterialPipeline>()
        {
            if let Some(gpu) = resources.remove::<ome_core::gpu::GpuContext>() {
                material_pipeline.sync_from_resources(gpu.queue(), resources);
                resources.insert(gpu);
            }
            resources.insert(material_pipeline);
        }

        let referenced = self.pipeline.collect_referenced_guids(resources);
        let pending: Vec<Guid> = referenced
            .iter()
            .copied()
            .filter(|guid| self.pipeline.lookup(*guid).is_none())
            .collect();
        if !referenced.is_empty() {
            tracing::debug!(
                target: "ome_render::meshlet::sync",
                referenced = referenced.len(),
                pending = pending.len(),
                cached = self.pipeline.registered_count(),
                "meshlet asset sync tick",
            );
        }
        if pending.is_empty() {
            return;
        }

        for guid in pending {
            // Take the AssetServer out so we can pass `resources`
            // (which holds `Assets<MeshletMesh>`) by &mut into the
            // load call. Re-insert before any continue/return so we
            // never leak the resource.
            let Some(mut server) = resources.remove::<AssetServer>() else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    "AssetServer resource missing; skipping meshlet asset sync",
                );
                return;
            };
            let load_result = server.load_by_guid::<MeshletMesh>(guid, resources);
            resources.insert(server);

            let handle = match load_result {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(
                        target: "ome_render::meshlet::sync",
                        guid = %guid,
                        error = %e,
                        "failed to load meshlet asset by GUID",
                    );
                    continue;
                }
            };

            let Some(assets) = resources.get::<Assets<MeshletMesh>>() else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    "Assets<MeshletMesh> resource missing after load; aborting sync",
                );
                return;
            };
            let Some(mesh) = assets.get(handle) else {
                tracing::warn!(
                    target: "ome_render::meshlet::sync",
                    guid = %guid,
                    "loaded handle resolved to empty Assets<MeshletMesh> entry",
                );
                continue;
            };

            self.ensure_gpu_mesh(device, guid, mesh);
            tracing::info!(
                target: "ome_render::meshlet::sync",
                guid = %guid,
                "uploaded meshlet asset to GPU",
            );
        }
    }
}
