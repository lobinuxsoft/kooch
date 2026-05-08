//! `pub fn render_with_assets` + `pub fn render` — the per-frame
//! orchestrator entry points.
//!
//! `render` is a thin dispatcher: it runs the prelude common to both
//! GPU paths (pool sync, ECS query, instance upload, growth checks,
//! per-frame bind groups, GPU timer slot, encoder) and then routes the
//! frame either through [`Self::render_path_r64`] (#493 atomic R64
//! vbuf, sibling file `render_r64.rs`) when the device supports it, or
//! through [`Self::render_path_hi_z_two_pass`] (legacy R32 + Hi-Z
//! 2-pass, sibling file `render_hi_z_2pass.rs`).
//!
//! Both extracted methods own their submits + readbacks and return
//! the [`MeshletRenderStats`] for the frame.

use glam::{Mat4, Vec3};

use ome_core::resource::Resources;

use crate::meshlet::cull::CullParams;
use crate::meshlet::debug::{MeshletDebugMode, MeshletLodSettings};
use crate::meshlet::gpu_meshlet::pool_meshlet_bind_group;
use crate::meshlet::scene::SceneCullParams;

use super::super::{MeshletRenderStage, MeshletRenderStats};

impl MeshletRenderStage {
    /// Records + submits one frame of the meshlet pipeline driven by
    /// `resources`'s ECS query against the multi-mesh `GpuGlobalMeshPool`.
    /// Lazy-rebuilds the GPU pool when [`Self::pool_dirty`] is set.
    ///
    /// Returns [`MeshletRenderStats::default`] when the pool has no
    /// registered mesh yet or the ECS query yielded no instances.
    pub fn render_with_assets(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        view_proj: Mat4,
        cam_pos: Vec3,
    ) -> MeshletRenderStats {
        if self.pool_dirty || self.gpu_pool.is_none() {
            if self.pipeline.registered_count() == 0 {
                return MeshletRenderStats::default();
            }
            self.gpu_pool = Some(self.pipeline.pool().upload(device));
            self.pool_dirty = false;
            tracing::debug!(
                target: "ome_render::meshlet::render",
                meshes = self.pipeline.registered_count(),
                meshlets = self.pipeline.pool().meshlets.len(),
                "rebuilt GpuGlobalMeshPool",
            );
        }
        self.render(device, queue, resources, view_proj, cam_pos)
    }

    /// Records + submits one frame against the current `gpu_pool`.
    /// Caller must have populated the pool via [`Self::ensure_gpu_mesh`]
    /// before invoking. Returns zero stats when the ECS query yields
    /// no instances; the stage does not clear in that case so the
    /// previous frame's color stays on the offscreen target.
    ///
    /// Takes `&mut self` because the cull dispatcher's
    /// `visible_meshlets` buffer is grown on demand to fit the
    /// scene's worst-case (instances × max_meshlets/mesh) thread
    /// count.
    ///
    /// # Draw-call accounting (#492)
    ///
    /// `MeshletRenderStats::draw_calls` reports **only the meshlet
    /// stage's contribution**:
    /// - `0` when the ECS query yields no instances (early return below
    ///   skips both the cull dispatch and every raster / deferred
    ///   submit).
    /// - `4` on the atomic R64 vbuf path (#493): cull + clear + raster
    ///   + deferred shade, all single-pass.
    /// - `6` on the legacy R32 + Hi-Z 2-pass path: cull A + raster A +
    ///   SPD pyramid build + cull B + raster B + deferred shade.
    ///
    /// The editor surface adds 3 fixed passes (sky background +
    /// viewport blit + egui paint) outside this stage; the perf-HUD
    /// `Draw calls / frame` field sums both contributions. An empty
    /// scene therefore reports `3`, not `0` — that is the editor base,
    /// not a leak.
    ///
    /// # Visibility filtering (#492)
    ///
    /// `MeshRenderer.visible == false` is filtered upstream at
    /// [`MeshletPipeline::collect_scene_instances`], so an invisible
    /// entity never enters the `Vec<MeshInstance>` and never reaches
    /// the cull dispatch. Same filter runs in
    /// [`MeshletPipeline::collect_referenced_guids`] so an invisible
    /// mesh is also not pulled into the GPU pool. There is no
    /// per-instance visibility flag inside the cull shader because the
    /// upstream filter makes one redundant.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &Resources,
        view_proj: Mat4,
        cam_pos: Vec3,
    ) -> MeshletRenderStats {
        // ── Prelude: shared between both GPU paths ─────────────────
        // Extract the per-frame `max_meshlets_per_mesh` immediately so
        // the `&self.gpu_pool` borrow is released before any `&mut
        // self` mutation (cull growth, scene upload). The path methods
        // re-borrow the pool internally for their dispatch calls.
        let max_meshlets_per_mesh = match self.gpu_pool.as_ref() {
            Some(pool) => pool.max_meshlets_per_mesh.max(1),
            None => {
                tracing::debug!(
                    target: "ome_render::meshlet::render",
                    "render skipped: gpu_pool not built yet",
                );
                return MeshletRenderStats::default();
            }
        };
        let instances = self.pipeline.collect_scene_instances(resources);
        if instances.is_empty() {
            tracing::debug!(
                target: "ome_render::meshlet::render",
                pipeline_registered = self.pipeline.registered_count(),
                "render skipped: zero instances",
            );
            return MeshletRenderStats::default();
        }
        tracing::debug!(
            target: "ome_render::meshlet::render",
            instances = instances.len(),
            "render dispatching meshlet pipeline",
        );
        assert!(
            (instances.len() as u32) <= self.instance_capacity,
            "MeshletRenderStage: collected {} instances exceeds capacity {}",
            instances.len(),
            self.instance_capacity,
        );

        self.scene.upload_instances(queue, &instances);
        // Worst-case meshlet stride covers every mesh; the pool path
        // bounds-checks per-instance against pool_mesh_descriptors.
        // (`max_meshlets_per_mesh` was bound from `gpu_pool` above so
        // the borrow released before the &mut-self upload.)
        // Approximate proj_scale_y by the absolute value of `view_proj.y_axis.y`.
        // For an ortho-normal view (look_at_rh with up=Y) the camera basis
        // contributes 1 to that component and the projection's `1 / tan(fovy/2)`
        // is preserved exactly. Skewed cameras pay a small error that the
        // 1-pixel target tolerance absorbs.
        let proj_scale_y = view_proj.y_axis.y.abs();
        let viewport_h_px = self.size.1 as f32;
        let lod_target = resources
            .get::<MeshletLodSettings>()
            .copied()
            .unwrap_or_default()
            .target_error_pixels
            .max(0.01);
        let debug_mode = resources
            .get::<MeshletDebugMode>()
            .copied()
            .unwrap_or_default()
            .as_u32();
        let cull_params = CullParams::new(view_proj, cam_pos, max_meshlets_per_mesh)
            .with_lod(viewport_h_px, proj_scale_y, lod_target)
            .with_debug_mode(debug_mode);
        let scene_params =
            SceneCullParams::new(instances.len() as u32, max_meshlets_per_mesh);

        // Grow visible_meshlets if the scene now needs more slots
        // than the dispatcher was sized for. Geometric growth absorbs
        // future jumps without per-frame reallocation.
        let required_capacity = scene_params
            .instance_count
            .saturating_mul(scene_params.meshlets_per_mesh);
        self.cull.ensure_capacity(device, required_capacity);
        // group_max_err sized to the per-instance prefix-sum total
        // (Σ over instances of mesh_descriptors[mesh_id].group_count),
        // not the pool's group_capacity. Per-mesh sizing collapsed
        // every instance of the same mesh into one slot range and
        // forced multi-instance LOD descent to the closest one's
        // verdict (#474). Same geometric-growth pattern as the
        // visible buffer.
        let required_group_capacity = self
            .pipeline
            .instance_group_capacity(&instances)
            .max(1);
        self.cull
            .ensure_group_capacity(device, required_group_capacity);

        // Build the meshlet + material bind groups. The `gpu_pool`
        // re-borrow lives only as long as `meshlet_bg` construction;
        // after that the path methods own &mut self exclusively.
        let meshlet_bg = {
            let gpu_pool = self.gpu_pool.as_ref().expect("checked above");
            pool_meshlet_bind_group(device, &self.meshlet_bgl, gpu_pool)
        };
        // Prefer the GUID-keyed `MaterialPipeline` pool when it's
        // present in resources — that's the buffer the asset picker
        // writes into. Falls back to the stage-local pool (used by
        // GPU integration tests that bypass the asset system).
        let material_bg = match resources.get::<crate::material::MaterialPipeline>() {
            Some(pipeline) => pipeline.pool().bind_group(device),
            None => self.material_pool.bind_group(device),
        };

        // #463.4 — drain any GPU timer slots that completed since
        // last frame, then acquire a fresh slot for this frame's
        // start/end timestamps. `None` means timers are disabled or
        // every ring slot is in flight; either way the render path
        // proceeds normally and the HUD keeps the previously-sampled
        // value.
        self.gpu_timers.drain_ready();
        let timer_slot = self.gpu_timers.acquire_slot();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("meshlet_render_stage_encoder"),
        });

        if timer_slot.is_some() {
            self.gpu_timers.write_start(&mut encoder);
        }

        let instance_count = instances.len() as u32;

        // ── Path switch ─────────────────────────────────────────────
        // Atomic R64 vbuf path (#493) when the device supports it;
        // otherwise the legacy R32 + Hi-Z 2-pass orchestrator.
        if self.vbuf64_stage.is_some() {
            return self.render_path_r64(
                device,
                queue,
                encoder,
                resources,
                view_proj,
                cam_pos,
                &cull_params,
                &scene_params,
                &meshlet_bg,
                &material_bg,
                timer_slot,
                instance_count,
            );
        }

        self.render_path_hi_z_two_pass(
            device,
            queue,
            encoder,
            resources,
            view_proj,
            cam_pos,
            &cull_params,
            &scene_params,
            &meshlet_bg,
            &material_bg,
            timer_slot,
            instance_count,
        )
    }
}
