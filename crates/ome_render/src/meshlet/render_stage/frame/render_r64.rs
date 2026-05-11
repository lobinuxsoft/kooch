//! Atomic R64 visibility-buffer path (#493) — single-pass cull → R64
//! winner-takes-all raster → deferred shade. Routed by
//! [`MeshletRenderStage::render`] when `vbuf64_stage` is `Some`.
//!
//! Owns the encoder it receives, writes the GPU timer end + readback,
//! submits, and returns the per-frame [`MeshletRenderStats`].

use glam::{Mat4, Vec3};

use ome_core::resource::Resources;

use crate::meshlet::cull::CullParams;
use crate::meshlet::debug::MeshletDebugMode;
use crate::meshlet::scene::SceneCullParams;

use super::super::{MeshletRenderStage, MeshletRenderStats};

impl MeshletRenderStage {
    /// Atomic R64 vbuf path. Same observable contract as the legacy
    /// path: 4 logical passes (cull + clear + raster + deferred) and
    /// stats reporting `draw_calls = 4`. See [`Self::render`] for the
    /// dispatch decision and the prelude that builds `cull_params` /
    /// `scene_params` / `meshlet_bg` / `material_bg` / `timer_slot`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_path_r64(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        mut encoder: wgpu::CommandEncoder,
        resources: &Resources,
        view_proj: Mat4,
        cam_pos: Vec3,
        cull_params: &CullParams,
        scene_params: &SceneCullParams,
        meshlet_bg: &wgpu::BindGroup,
        material_bg: &wgpu::BindGroup,
        timer_slot: Option<usize>,
        instance_count: u32,
    ) -> MeshletRenderStats {
        let gpu_pool = self.gpu_pool.as_ref().expect("checked by render() prelude");
        let vbuf64 = self
            .vbuf64_stage
            .as_ref()
            .expect("path selected only when vbuf64_stage is Some");

        self.cull.dispatch_scene_pool_atomic(
            device,
            queue,
            &mut encoder,
            gpu_pool,
            &self.scene,
            cull_params,
            scene_params,
        );
        let debug_mode = resources
            .get::<MeshletDebugMode>()
            .copied()
            .unwrap_or_default();
        // #454: the modal accumulator runs in TriangleDensity /
        // Overdraw modes (and falls back to the TriangleDensity
        // count for the reject overlay modes still being wired in
        // later subtasks). Production rendering (`Off`, `MeshletIds`,
        // `InstanceIds`, `CullPassthrough`, `OnlyLod0`, `OnlyRoots`)
        // leaves the uniform at 0 so the hot path skips the atomic
        // increment. The density texture is allocated in lockstep
        // with the stage's render targets; it is `Some` whenever
        // the vbuf64 path is selectable.
        let density_mode: u32 = match debug_mode {
            MeshletDebugMode::TriangleDensity
            | MeshletDebugMode::HiZRejected
            | MeshletDebugMode::BackfaceRejected => 1,
            MeshletDebugMode::Overdraw => 2,
            _ => 0,
        };
        let density_view = self
            .triangle_density_view
            .as_ref()
            .expect("density texture must be allocated whenever vbuf64 path is active");
        vbuf64.render(
            device,
            queue,
            &mut encoder,
            &self.depth_view,
            &self.color_view,
            density_view,
            density_mode,
            meshlet_bg,
            material_bg,
            &self.cull,
            &self.scene,
            view_proj,
            debug_mode.as_u32(),
            /* clear_depth */ true,
        );

        if let Some(slot_idx) = timer_slot {
            self.gpu_timers.write_end_and_copy(&mut encoder, slot_idx);
        }
        queue.submit(std::iter::once(encoder.finish()));
        if let Some(slot_idx) = timer_slot {
            self.gpu_timers.submit_readback(slot_idx);
        }

        let pool = self.pipeline.pool();
        let pool_meshlets_total = pool.meshlets.len() as u32;
        let pool_meshlets_roots = pool
            .meshlets
            .iter()
            .filter(|m| m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT)
            .count() as u32;
        // R64 path: cull + clear + raster + deferred.
        let meshlet_draw_calls = if instance_count == 0 { 0 } else { 4 };
        MeshletRenderStats {
            instances_uploaded: instance_count,
            cull_threads: scene_params.instance_count * scene_params.meshlets_per_mesh,
            cam_pos: cam_pos.to_array(),
            pool_meshlets_total,
            pool_meshlets_roots,
            gpu_frame_ms: self.gpu_timers.last_frame_ms(),
            draw_calls: meshlet_draw_calls,
        }
    }
}
