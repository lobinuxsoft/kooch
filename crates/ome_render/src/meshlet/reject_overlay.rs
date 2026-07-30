//! Reject-reason debug overlay (#454.4).
//!
//! Compute pass that reads the per-thread `reject_reasons[]` written
//! by `cs_cull_scene_pool_atomic` and rasterises a 1-pixel wireframe
//! rectangle around every meshlet whose reject reason matches the
//! host-supplied `selected_reason`. The overlay writes through the
//! deferred shader's existing colour storage texture binding — no
//! `RENDER_ATTACHMENT` usage bump or alpha-blend pass needed.
//!
//! # Bind-group reuse
//!
//! - `group(0)` is overlay-private: a small UBO with view_proj +
//!   screen size + the selected reason + line thickness, plus the
//!   colour storage-texture target.
//! - `group(1) … group(3)` reuse the layouts already exposed by
//!   [`MeshletCull`] so the overlay sees the same pool / scene /
//!   reject-reason buffers the cull pass writes through. Avoids
//!   duplicating BGLs and keeps the dispatcher the single source of
//!   truth for those handles.
//!
//! # Activation
//!
//! Owned by [`MeshletRenderStage`] as `Option<MeshletRejectOverlay>`,
//! `Some` only when `MeshletDebugCaps::supports_texture_atomic` is
//! `true` — the same gate the triangle-density / overdraw heatmaps
//! use, since both features ride the same baseline-vs-pre-baseline
//! split. The orchestrator dispatches the overlay only when the
//! current frame's `MeshletDebugMode` selects a reject-reason mode
//! AND `cull_params.debug_active` was set so the SSBO actually
//! carries this frame's reasons.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use super::deferred::DEFERRED_COLOR_FORMAT;
use super::dispatcher::MeshletCull;
use super::scene::MeshletScene;

const SHADER_SOURCE: &str = include_str!("../../shaders/meshlet_reject_overlay.wgsl");

/// Reject-reason discriminant the cull shader writes per thread —
/// matches the `REJECT_REASON_*` constants in
/// `meshlet_cull/atomic.wgsl`. Re-exported as a stable enum so the
/// render stage can map [`super::debug::MeshletDebugMode`] variants
/// to the SSBO codes without touching shader literals.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RejectReason {
    Skipped = 0,
    Passed = 1,
    Frustum = 2,
    Backface = 3,
    HiZ = 4,
    Lod = 5,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default, Pod, Zeroable)]
struct OverlayParams {
    view_proj: [[f32; 4]; 4],
    screen_size: [u32; 2],
    selected_reason: u32,
    line_thickness_px: u32,
}

/// Owns the overlay's compute pipeline + its private UBO. The
/// host-side bind groups are built per dispatch because the colour
/// view + scene buffers + cull buffers can be swapped between
/// frames (resize, scene-pool rebuild, ensure_capacity).
pub struct MeshletRejectOverlay {
    pipeline: wgpu::ComputePipeline,
    overlay_bgl: wgpu::BindGroupLayout,
    params_buffer: wgpu::Buffer,
}

impl MeshletRejectOverlay {
    /// Builds the overlay pipeline. The pipeline_layout reuses the
    /// pool / scene / debug BGLs exposed by [`MeshletCull`] so a
    /// single `MeshletCull` handle drives both the cull writes and
    /// the overlay reads.
    pub fn new(device: &wgpu::Device, cull: &MeshletCull) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_reject_overlay_shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let overlay_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("meshlet_reject_overlay_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<OverlayParams>() as u64
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: DEFERRED_COLOR_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_reject_overlay_pipeline_layout"),
            bind_group_layouts: &[
                Some(&overlay_bgl),
                Some(cull.pool_bind_group_layout()),
                Some(cull.scene_bind_group_layout()),
                Some(cull.debug_bind_group_layout()),
            ],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_reject_overlay_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_reject_overlay"),
            compilation_options: Default::default(),
            cache: None,
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_reject_overlay_params"),
            contents: bytemuck::bytes_of(&OverlayParams::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            pipeline,
            overlay_bgl,
            params_buffer,
        }
    }

    /// Records the overlay compute pass into `encoder`. Builds bind
    /// groups against the current frame's targets each call because
    /// resize / pool rebuild / ensure_capacity may have invalidated
    /// the previous frame's handles.
    ///
    /// `total_threads` MUST equal `instance_count × meshlets_per_mesh`
    /// — the same dispatch shape the cull pass used to populate
    /// `reject_reasons[]`. A mismatched count either over-iterates
    /// (reads garbage) or under-iterates (skips clusters); both are
    /// silent visual bugs in the overlay.
    #[allow(clippy::too_many_arguments)]
    pub fn dispatch(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        cull: &MeshletCull,
        scene: &MeshletScene,
        pool: &super::pool::GpuGlobalMeshPool,
        view_proj: Mat4,
        screen_size: (u32, u32),
        selected_reason: RejectReason,
        line_thickness_px: u32,
        total_threads: u32,
    ) {
        if total_threads == 0 || screen_size.0 == 0 || screen_size.1 == 0 {
            return;
        }

        let params = OverlayParams {
            view_proj: view_proj.to_cols_array_2d(),
            screen_size: [screen_size.0, screen_size.1],
            selected_reason: selected_reason as u32,
            line_thickness_px: line_thickness_px.max(1),
        };
        queue.write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&params));

        let overlay_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_reject_overlay_bg"),
            layout: &self.overlay_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
            ],
        });
        let pool_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_reject_overlay_pool_bg"),
            layout: cull.pool_bind_group_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: pool.mesh_descriptors.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: pool.meshlets.as_entire_binding(),
                },
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_reject_overlay_scene_bg"),
            layout: cull.scene_bind_group_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: cull.scene_params_buffer().as_entire_binding(),
                },
            ],
        });
        // debug_bgl gained binding(1) for stage_counters in #454.6.
        // The overlay shader only references reject_reasons, but
        // wgpu requires every BGL entry to be present in the bind
        // group at construction — even when the shader doesn't
        // sample the bound resource. Providing stage_counters here
        // is a pure table write; the GPU never accesses it from this
        // pipeline.
        let debug_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_reject_overlay_debug_bg"),
            layout: cull.debug_bind_group_layout(),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cull.reject_reasons_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: cull.stage_counters_buffer().as_entire_binding(),
                },
            ],
        });

        let workgroups = total_threads.div_ceil(64).max(1);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("meshlet_reject_overlay_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &overlay_bg, &[]);
        pass.set_bind_group(1, &pool_bg, &[]);
        pass.set_bind_group(2, &scene_bg, &[]);
        pass.set_bind_group(3, &debug_bg, &[]);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(SHADER_SOURCE)
            .expect("meshlet_reject_overlay.wgsl should parse");
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .expect("meshlet_reject_overlay.wgsl should validate");
    }

    #[test]
    fn overlay_params_layout_is_pod() {
        // 64-byte mat4 + 8-byte vec2u + 4-byte selected + 4-byte
        // thickness = 80 B. Multiple of 16 — std140-friendly.
        assert_eq!(std::mem::size_of::<OverlayParams>(), 80);
    }

    #[test]
    fn reject_reason_discriminants_match_shader() {
        // The cull shader's `REJECT_REASON_*` constants pin these
        // values. Reordering breaks the overlay's mode → reason
        // lookup silently — test fails first.
        assert_eq!(RejectReason::Skipped as u32, 0);
        assert_eq!(RejectReason::Passed as u32, 1);
        assert_eq!(RejectReason::Frustum as u32, 2);
        assert_eq!(RejectReason::Backface as u32, 3);
        assert_eq!(RejectReason::HiZ as u32, 4);
        assert_eq!(RejectReason::Lod as u32, 5);
    }
}
