//! GPU mesh cache upkeep + ECS asset sync for [`MeshletRenderStage`].
//!
//! - [`Self::ensure_gpu_mesh`] registers a single [`MeshletMesh`] under
//!   a GUID into the pool (idempotent).
//! - [`Self::gpu_mesh_count`] reports the current registry size.
//! - [`Self::sync_assets_to_gpu`] resolves every visible
//!   `MeshRenderer.mesh` GUID through the [`AssetServer`] and uploads
//!   any that aren't yet GPU-resident. Also runs the per-frame material
//!   pool sync prior to cull dispatch.

use kooch_core::Guid;
use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;

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
    pub fn ensure_gpu_mesh(&mut self, _device: &wgpu::Device, guid: Guid, mesh: &MeshletMesh) {
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
    pub fn sync_assets_to_gpu(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut Resources,
    ) {
        // Material pool sync first: the meshlet scene system reads
        // `MaterialPipeline.lookup_or_fallback` when assembling
        // `MeshInstance.material_id`, so any newly-picked GUID has
        // to be in the registry before the cull dispatch fires.
        // `queue` is now an explicit parameter so the caller never
        // has to leave `GpuContext` in `Resources` while we're here —
        // the editor render system removes it for the whole frame, and
        // the previous in-method `resources.remove::<GpuContext>()`
        // returned `None` silently in that path, dropping every
        // material picked through the inspector (bug #533).
        if let Some(mut material_pipeline) = resources.remove::<crate::material::MaterialPipeline>()
        {
            material_pipeline.sync_from_resources(device, queue, resources);
            resources.insert(material_pipeline);
        } else {
            tracing::debug!(
                target: "kooch_render::material::sync",
                "sync_assets_to_gpu: MaterialPipeline absent from Resources; material sync skipped",
            );
        }

        // Meshes built this frame reach the pool before anything is
        // looked up on disk, so a generated GUID never counts as pending
        // and never sends the AssetServer after a file that does not
        // exist.
        if let Some(mut generated) = resources.remove::<crate::meshlet::GeneratedMeshes>() {
            for (guid, mesh) in generated.drain() {
                self.ensure_gpu_mesh(device, guid, &mesh);
            }
            resources.insert(generated);
        }

        let referenced = self.pipeline.collect_referenced_guids(resources);
        let pending: Vec<Guid> = referenced
            .iter()
            .copied()
            .filter(|guid| self.pipeline.lookup(*guid).is_none())
            // 🔴 Only GUIDs a mesh loader can read. A generated mesh is
            // named by the GUID of the file it was generated FROM — a
            // block's `.block` — and asking the server for it produces
            // "loader does not support extension", once per such GUID,
            // for a file that was never going to be a mesh. What draws
            // it is the drain above.
            .filter(|guid| reads_as_mesh(resources, *guid))
            .collect();
        if !referenced.is_empty() {
            tracing::debug!(
                target: "kooch_render::meshlet::sync",
                referenced = referenced.len(),
                pending = pending.len(),
                cached = self.pipeline.registered_count(),
                "meshlet asset sync tick",
            );
        }
        // Dropped the moment a GUID stops being pending, so a mesh that
        // comes back and breaks again is reported again.
        self.unresolved.retain(|guid| pending.contains(guid));

        if pending.is_empty() {
            return;
        }

        // 🔴 Every mesh in the scene failing is not N warnings, it is one
        // broken run — and the actionable line was buried under a
        // thousand correct ones. Said once, when the whole set is
        // unresolved and none of it has been reported yet.
        let all_broken = pending.len() == referenced.len() && self.unresolved.is_empty();
        if all_broken && self.pipeline.registered_count() == 0 {
            tracing::error!(
                target: "kooch_render::meshlet::sync",
                meshes = referenced.len(),
                "not one mesh in this scene resolves, so nothing will draw. \
                 The asset database is empty — a game started outside the \
                 editor needs KOOCH_ENGINE_ROOT to find the engine's own assets",
            );
        }

        for guid in pending {
            // Take the AssetServer out so we can pass `resources`
            // (which holds `Assets<MeshletMesh>`) by &mut into the
            // load call. Re-insert before any continue/return so we
            // never leak the resource.
            let Some(mut server) = resources.remove::<AssetServer>() else {
                tracing::warn!(
                    target: "kooch_render::meshlet::sync",
                    "AssetServer resource missing; skipping meshlet asset sync",
                );
                return;
            };
            let load_result = server.load_by_guid::<MeshletMesh>(guid, resources);
            resources.insert(server);

            let handle = match load_result {
                Ok(h) => h,
                Err(e) => {
                    // `insert` answers "was this new" — the retry stays
                    // every frame, because an asset can arrive late; only
                    // the saying stops.
                    if self.unresolved.insert(guid) {
                        tracing::warn!(
                            target: "kooch_render::meshlet::sync",
                            guid = %guid,
                            error = %e,
                            "failed to load meshlet asset by GUID; \
                             said once until it resolves",
                        );
                    }
                    continue;
                }
            };

            let Some(assets) = resources.get::<Assets<MeshletMesh>>() else {
                tracing::warn!(
                    target: "kooch_render::meshlet::sync",
                    "Assets<MeshletMesh> resource missing after load; aborting sync",
                );
                return;
            };
            let Some(mesh) = assets.get(handle) else {
                tracing::warn!(
                    target: "kooch_render::meshlet::sync",
                    guid = %guid,
                    "loaded handle resolved to empty Assets<MeshletMesh> entry",
                );
                continue;
            };

            self.ensure_gpu_mesh(device, guid, mesh);
            tracing::info!(
                target: "kooch_render::meshlet::sync",
                guid = %guid,
                "uploaded meshlet asset to GPU",
            );
        }
    }
}

/// Whether this GUID names a file a mesh loader can read.
///
/// Unregistered or untyped answers `true`: the type lands on the entry
/// the first time something loads it, so refusing earlier would stop a
/// mesh from ever being read.
fn reads_as_mesh(resources: &Resources, guid: Guid) -> bool {
    let Some(type_name) = resources
        .get::<kooch_core::asset_database::AssetDatabase>()
        .and_then(|db| db.entry(guid).and_then(|entry| entry.type_name.clone()))
    else {
        return true;
    };
    type_name == std::any::type_name::<crate::mesh::Mesh>()
        || type_name == std::any::type_name::<MeshletMesh>()
}
