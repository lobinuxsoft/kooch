use crate::meshlet::gpu_meshlet::meshlet_bind_group_layout;
use crate::meshlet::scene::MeshletScene;

use super::super::pipelines::MeshletCullPipelines;
use super::CULL_SHADER_SOURCE;
use super::bgls::*;

impl MeshletCullPipelines {
    /// Compiles the cull shader and builds every pipeline + layout.
    ///
    /// Called once per render stage, not once per view: nine compute
    /// pipelines per camera is what this split exists to avoid.
    pub fn new(device: &wgpu::Device) -> Self {
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

        // Two-level cull (#1002). All four entries share one layout:
        // the cull group(0), the pool group(1), the scene group(2),
        // `group_max_err` + bounds + chunk list at group(3) and the
        // debug buffers at group(4) — the same five the rectangle
        // entries bind, so one set of bind groups drives every pass.
        let chunked_bgl = build_chunked_bgl(device);
        let pipeline_layout_chunked =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("meshlet_cull_chunked_pipeline_layout"),
                bind_group_layouts: &[
                    Some(&cull_bgl),
                    Some(&pool_bgl),
                    Some(&scene_bgl),
                    Some(&chunked_bgl),
                    Some(&debug_bgl),
                ],
                immediate_size: 0,
            });
        let chunked = |label: &str, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout_chunked),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let pipeline_cull_instances =
            chunked("meshlet_cull_instances_pipeline", "cs_cull_instances");
        let pipeline_cull_expand_args =
            chunked("meshlet_cull_expand_args_pipeline", "cs_cull_expand_args");
        let pipeline_lod_group_max_err_chunked = chunked(
            "meshlet_lod_group_max_err_chunked_pipeline",
            "cs_lod_group_max_err_chunked",
        );
        let pipeline_cull_scene_pool_atomic_chunked = chunked(
            "meshlet_cull_scene_pool_atomic_chunked_pipeline",
            "cs_cull_scene_pool_atomic_chunked",
        );

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
            pipeline_cull_instances,
            pipeline_cull_expand_args,
            pipeline_lod_group_max_err_chunked,
            pipeline_cull_scene_pool_atomic_chunked,
            cull_bgl,
            extended_cull_bgl,
            hi_z_bgl,
            scene_bgl,
            scene_with_hi_z_bgl,
            meshlet_bgl,
            pool_bgl,
            group_err_bgl,
            debug_bgl,
            chunked_bgl,
        }
    }
}
