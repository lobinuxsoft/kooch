use wgpu::util::DeviceExt;

use crate::meshlet::cull::CullParams;
use crate::meshlet::scene::SceneCullParams;

use super::super::MeshletCull;
use super::super::types::{DrawIndirectArgs, HiZTestParams};

impl MeshletCull {
    /// Allocates one view's cull buffers, sized for at most `capacity`
    /// visible meshlets per frame. `max_triangles_per_meshlet` controls
    /// the fixed `vertex_count` used by the indirect draw — must match
    /// the builder's setting (default `meshlet::DEFAULT_MAX_TRIANGLES`).
    ///
    /// Pipelines are **not** built here — they live in
    /// [`super::super::pipelines::MeshletCullPipelines`], one per stage
    /// rather than one per view.
    pub fn new(device: &wgpu::Device, capacity: u32, max_triangles_per_meshlet: u32) -> Self {
        assert!(capacity > 0, "MeshletCull capacity must be non-zero");

        // A ring, not one struct — see `PARAMS_RING`. The stride is the
        // device's uniform offset alignment because a bind group binds
        // a sub-range by byte offset and the API requires that.
        let params_stride = (device.limits().min_uniform_buffer_offset_alignment as u64)
            .max(std::mem::size_of::<CullParams>() as u64);
        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_params"),
            size: params_stride * super::super::PARAMS_RING,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hi_z_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_hi_z_params"),
            size: std::mem::size_of::<HiZTestParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Per view even though its contents do not depend on the
        // camera: two views of the same scene would write identical
        // bytes, so sharing it would work today and break silently the
        // first time two views render different scenes. 16 bytes is not
        // worth an exception to the rule.
        let scene_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_scene_params"),
            size: std::mem::size_of::<SceneCullParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let visible_meshlets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_visible_ids"),
            size: capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let visible_count = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_visible_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Hi-Z 2-pass cull reject queue (#445). Worst-case capacity
        // matches the visible buffer (every meshlet occluded). Pass A
        // appends; pass B drains via the atomic counter.
        let culled_meshlets = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_culled_ids"),
            size: capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let culled_count = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_culled_count"),
            size: 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::INDIRECT,
            mapped_at_creation: false,
        });

        // Initial group_max_err buffer. Sized to a small power of
        // two; the dispatcher grows it geometrically when a scene's
        // group_capacity exceeds this. Storage + COPY_DST so we can
        // clear it each frame before pass 1 of the 2-pass cull.
        //
        // Per view: the LOD error it accumulates is measured in pixels,
        // so it depends on where the camera is and how wide its
        // projection is. Sharing it would make a shadow cascade pick
        // the main view's LOD.
        let initial_group_capacity: u32 = 256;
        let group_max_err = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_group_max_err"),
            size: initial_group_capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Per-thread reject_reasons buffer (#454.4). Sized to the
        // same `capacity` as `visible_meshlets` because it is
        // indexed by the cull thread id, which equals
        // `instance_count × meshlets_per_mesh` — exactly the same
        // dispatch shape that drives the visible-output buffer.
        // `ensure_capacity` recreates both in lock-step.
        let reject_reasons = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_reject_reasons"),
            size: capacity as u64 * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // Per-stage cull survivor counters (#454.6). 4 × u32 = 16 B,
        // atomicAdded by the cull shader at each stage tail when
        // `CullParams.debug_active != 0`. Cleared each frame; readback
        // drives the editor's stats overlay row. Per view, or the
        // overlay reports one camera's survivors under another's name.
        let stage_counters = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_stage_counters"),
            size: 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vertex_count_per_instance = max_triangles_per_meshlet * 3;
        let indirect_args = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("meshlet_indirect_args"),
            contents: bytemuck::bytes_of(&DrawIndirectArgs {
                vertex_count: vertex_count_per_instance,
                instance_count: 0,
                first_vertex: 0,
                first_instance: 0,
            }),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            params_buffer,
            hi_z_params_buffer,
            scene_params_buffer,
            visible_meshlets,
            visible_count,
            culled_meshlets,
            culled_count,
            indirect_args,
            group_max_err,
            group_capacity: initial_group_capacity,
            reject_reasons,
            stage_counters,
            capacity,
            vertex_count_per_instance,
            params_stride,
            params_cursor: std::sync::atomic::AtomicU32::new(0),
        }
    }
}
