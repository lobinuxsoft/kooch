use wgpu::util::DeviceExt;

use crate::meshlet::cull::CullParams;
use crate::meshlet::gpu_meshlet::meshlet_bind_group_layout;
use crate::meshlet::scene::{MeshletScene, SceneCullParams};

use super::super::MeshletCull;
use super::super::types::{DrawIndirectArgs, HiZTestParams};
use super::CULL_SHADER_SOURCE;
use super::bgls::*;

impl MeshletCull {
    /// Creates a dispatcher sized for at most `capacity` visible
    /// meshlets per frame. `max_triangles_per_meshlet` controls the
    /// fixed `vertex_count` used by the indirect draw — must match the
    /// builder's setting (default `meshlet::DEFAULT_MAX_TRIANGLES`).
    pub fn new(device: &wgpu::Device, capacity: u32, max_triangles_per_meshlet: u32) -> Self {
        assert!(capacity > 0, "MeshletCull capacity must be non-zero");

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("meshlet_cull_shader"),
            source: wgpu::ShaderSource::Wgsl(CULL_SHADER_SOURCE.into()),
        });

        let cull_bgl = build_cull_bgl(device);
        let extended_cull_bgl = build_extended_cull_bgl(device);
        let hi_z_bgl = build_hi_z_bgl(device);
        let scene_bgl = MeshletScene::bind_group_layout(device);
        let scene_with_hi_z_bgl = build_scene_with_hi_z_bgl(device);
        let meshlet_bgl = meshlet_bind_group_layout(device);
        // Two-binding subset of the pool BGL — keeps the cull
        // entry under the wgpu max_storage_buffers_per_shader_stage
        // limit (8). The full 5-binding pool BGL is used by the
        // rasterizer + deferred where storage-buffer headroom is
        // larger and vertex/triangle buffers are needed.
        let pool_bgl = build_cull_pool_bgl(device);
        let group_err_bgl = build_group_err_bgl(device);
        let debug_bgl = build_debug_bgl(device);

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_cull_pipeline_layout"),
            bind_group_layouts: &[Some(&cull_bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_cull_pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_cull"),
            compilation_options: Default::default(),
            cache: None,
        });

        let pipeline_layout_hi_z = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("meshlet_cull_hi_z_pipeline_layout"),
            bind_group_layouts: &[Some(&cull_bgl), Some(&hi_z_bgl)],
            immediate_size: 0,
        });
        let pipeline_hi_z = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_cull_hi_z_pipeline"),
            layout: Some(&pipeline_layout_hi_z),
            module: &shader,
            entry_point: Some("cs_cull_hi_z"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Scene-wide cull pipeline (`cs_cull_scene`) — group(0) cull
        // shared with the per-mesh path, group(2) instance buffer +
        // SceneCullParams. group(1) is unused here so the layout array
        // marks it None.
        let pipeline_layout_scene =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meshlet_cull_scene_pipeline_layout"),
                bind_group_layouts: &[Some(&cull_bgl), None, Some(&scene_bgl)],
                immediate_size: 0,
            });
        let pipeline_scene = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("meshlet_cull_scene_pipeline"),
            layout: Some(&pipeline_layout_scene),
            module: &shader,
            entry_point: Some("cs_cull_scene"),
            compilation_options: Default::default(),
            cache: None,
        });

        // Multi-mesh scene cull (`cs_cull_scene_pool`) — group(0) cull
        // shared with the per-mesh path (the `descriptors` binding at
        // group(0)@1 stays bound but the entry point ignores it),
        // group(1) the GlobalMeshPool, group(2) the scene buffers.
        let pipeline_layout_scene_pool =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meshlet_cull_scene_pool_pipeline_layout"),
                bind_group_layouts: &[Some(&cull_bgl), Some(&pool_bgl), Some(&scene_bgl)],
                immediate_size: 0,
            });
        let pipeline_scene_pool =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meshlet_cull_scene_pool_pipeline"),
                layout: Some(&pipeline_layout_scene_pool),
                module: &shader,
                entry_point: Some("cs_cull_scene_pool"),
                compilation_options: Default::default(),
                cache: None,
            });

        // 2-pass cull (#465 + #454.4). Both entries share the same
        // pipeline layout: cull group(0), pool group(1), scene
        // group(2), group_err group(3), debug-reject group(4). Pass 1
        // atomicMaxes pixel error per group_index; pass 2 reads the
        // same buffer to drive group-atomic descent decisions and
        // (when `params.debug_active != 0`) writes per-thread reject
        // reasons into `reject_reasons[]` for the overlay raster
        // pass to colourise.
        //
        // Group(4)'s `reject_reasons` SSBO is only referenced by the
        // cull entry; naga emits no requirement for it from
        // `cs_lod_compute_group_max_err`. The dispatcher binds it for
        // both passes anyway because the pipeline layout is shared
        // and the bind cost is a single table write per pass.
        let pipeline_layout_atomic =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_pipeline_layout"),
                bind_group_layouts: &[
                    Some(&cull_bgl),
                    Some(&pool_bgl),
                    Some(&scene_bgl),
                    Some(&group_err_bgl),
                    Some(&debug_bgl),
                ],
                immediate_size: 0,
            });
        let pipeline_lod_compute_group_max_err =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meshlet_lod_compute_group_max_err_pipeline"),
                layout: Some(&pipeline_layout_atomic),
                module: &shader,
                entry_point: Some("cs_lod_compute_group_max_err"),
                compilation_options: Default::default(),
                cache: None,
            });
        let pipeline_cull_scene_pool_atomic =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_pipeline"),
                layout: Some(&pipeline_layout_atomic),
                module: &shader,
                entry_point: Some("cs_cull_scene_pool_atomic"),
                compilation_options: Default::default(),
                cache: None,
            });

        // Hi-Z 2-pass cull (#445). Same shader, but pass 1 + pass A
        // bind under an extended layout: cull group(0) gains
        // culled_meshlets + culled_count, scene group(2) gains
        // hi_z_params + pyramid texture. Pass 1 (`cs_lod_compute_*`)
        // recompiles against this layout because both passes are
        // dispatched in lock-step from `dispatch_scene_pool_atomic_hi_z`
        // and binding the same cull bind group between them is the
        // path of least resistance.
        let pipeline_layout_atomic_hi_z =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_hi_z_pipeline_layout"),
                bind_group_layouts: &[
                    Some(&extended_cull_bgl),
                    Some(&pool_bgl),
                    Some(&scene_with_hi_z_bgl),
                    Some(&group_err_bgl),
                ],
                immediate_size: 0,
            });
        let pipeline_lod_compute_group_max_err_hi_z =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meshlet_lod_compute_group_max_err_hi_z_pipeline"),
                layout: Some(&pipeline_layout_atomic_hi_z),
                module: &shader,
                entry_point: Some("cs_lod_compute_group_max_err"),
                compilation_options: Default::default(),
                cache: None,
            });
        let pipeline_cull_scene_pool_atomic_hi_z =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meshlet_cull_scene_pool_atomic_hi_z_pipeline"),
                layout: Some(&pipeline_layout_atomic_hi_z),
                module: &shader,
                entry_point: Some("cs_cull_scene_pool_atomic_hi_z"),
                compilation_options: Default::default(),
                cache: None,
            });
        let pipeline_cull_pass_b =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("meshlet_cull_pass_b_pipeline"),
                layout: Some(&pipeline_layout_atomic_hi_z),
                module: &shader,
                entry_point: Some("cs_cull_pass_b"),
                compilation_options: Default::default(),
                cache: None,
            });

        let params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_params"),
            size: std::mem::size_of::<CullParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hi_z_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("meshlet_cull_hi_z_params"),
            size: std::mem::size_of::<HiZTestParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
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
        // drives the editor's stats overlay row.
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
            pipeline,
            pipeline_hi_z,
            pipeline_scene,
            pipeline_scene_pool,
            pipeline_lod_compute_group_max_err,
            pipeline_cull_scene_pool_atomic,
            pipeline_lod_compute_group_max_err_hi_z,
            pipeline_cull_scene_pool_atomic_hi_z,
            pipeline_cull_pass_b,
            cull_bgl,
            extended_cull_bgl,
            hi_z_bgl,
            scene_bgl,
            scene_with_hi_z_bgl,
            meshlet_bgl,
            pool_bgl,
            group_err_bgl,
            debug_bgl,
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
        }
    }
}
