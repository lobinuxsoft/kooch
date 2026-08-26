//! The four passes that build the grid, and the pipelines they run.
//!
//! Order matters and every step depends on the one before it:
//!
//! ```text
//!   z_slice   compute  one thread per light  ->  work list + draw args
//!   finalize  compute  one thread            ->  clamp the draw args
//!   count     raster   one fragment per pair ->  how many per cell
//!   allocate  compute  prefix sum            ->  where each run starts
//!   populate  raster   one fragment per pair ->  the indices themselves
//! ```
//!
//! Each pass is its own encoder pass, which is also what orders them:
//! wgpu inserts the barrier between passes, so nothing here has to.

use super::buffers::ClusterBuffers;

const COMMON: &str = include_str!("../../shaders/cluster_common.wgsl");
const Z_SLICE: &str = include_str!("../../shaders/cluster_z_slice.wgsl");
const ALLOCATE: &str = include_str!("../../shaders/cluster_allocate.wgsl");
const RASTER: &str = include_str!("../../shaders/cluster_raster.wgsl");

/// Placeholder the rasterizer carries where its pass kind goes.
const POPULATE_PLACEHOLDER: &str = "{{CLUSTER_POPULATE}}";

/// Threads per workgroup in the z-slice pass. Mirrors its
/// `@workgroup_size`.
const Z_SLICE_GROUP: u32 = 64;
/// Cells one allocation block covers. Mirrors its `@workgroup_size`, and
/// 256 is wgpu's ceiling rather than a tuning choice — it is the reason
/// the prefix sum takes two dispatches.
const ALLOCATE_BLOCK: u32 = 256;

/// The pipelines, their layouts, and the colour target the rasterizer
/// needs but never writes.
pub(super) struct ClusterPasses {
    build_layout: wgpu::BindGroupLayout,
    raster_layout: wgpu::BindGroupLayout,
    z_slice: wgpu::ComputePipeline,
    finalize: wgpu::ComputePipeline,
    allocate_local: wgpu::ComputePipeline,
    allocate_global: wgpu::ComputePipeline,
    count: wgpu::RenderPipeline,
    populate: wgpu::RenderPipeline,
    /// Sized to the grid: the rasterizer's viewport comes from its
    /// attachment, and the attachment is what makes a fragment a cell.
    target: Option<(wgpu::TextureView, u32, u32)>,
}

impl ClusterPasses {
    pub fn new(device: &wgpu::Device) -> Self {
        let build_layout = build_layout(device);
        let raster_layout = raster_layout(device);

        let build_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cluster_build_pipeline_layout"),
                bind_group_layouts: &[Some(&build_layout)],
                immediate_size: 0,
            });
        let raster_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cluster_raster_pipeline_layout"),
                bind_group_layouts: &[Some(&raster_layout)],
                immediate_size: 0,
            });

        let z_slice_module = module(device, "cluster_z_slice", Z_SLICE);
        let allocate_module = module(device, "cluster_allocate", ALLOCATE);
        // Two modules from one source, the substitution deciding which
        // half of the rasterizer compiles. A runtime flag would be one
        // module and a branch per fragment; this is the mechanism
        // `inti_pbr_shader` already uses for the same reason.
        let count_module = module(
            device,
            "cluster_raster_count",
            &RASTER.replace(POPULATE_PLACEHOLDER, "false"),
        );
        let populate_module = module(
            device,
            "cluster_raster_populate",
            &RASTER.replace(POPULATE_PLACEHOLDER, "true"),
        );

        Self {
            z_slice: compute(
                device,
                &build_pipeline_layout,
                &z_slice_module,
                "z_slice_main",
            ),
            finalize: compute(
                device,
                &build_pipeline_layout,
                &z_slice_module,
                "finalize_main",
            ),
            allocate_local: compute(
                device,
                &build_pipeline_layout,
                &allocate_module,
                "allocate_local_main",
            ),
            allocate_global: compute(
                device,
                &build_pipeline_layout,
                &allocate_module,
                "allocate_global_main",
            ),
            count: raster(
                device,
                &raster_pipeline_layout,
                &count_module,
                "cluster_count",
            ),
            populate: raster(
                device,
                &raster_pipeline_layout,
                &populate_module,
                "cluster_populate",
            ),
            build_layout,
            raster_layout,
            target: None,
        }
    }

    /// Points the rasterizer at a target the size of the grid, building
    /// one if the grid changed shape.
    pub fn ensure_target(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if matches!(self.target, Some((_, w, h)) if w == width && h == height) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cluster_raster_target"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.target = Some((texture.create_view(&Default::default()), width, height));
    }

    pub fn build_bind_group(
        &self,
        device: &wgpu::Device,
        buffers: &ClusterBuffers,
        lights: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cluster_build_bg"),
            layout: &self.build_layout,
            entries: &[
                entry(0, buffers.view.as_entire_binding()),
                entry(1, lights.as_entire_binding()),
                entry(2, buffers.draw.as_entire_binding()),
                entry(3, buffers.work_list.as_entire_binding()),
                entry(4, buffers.cells.as_entire_binding()),
                entry(5, buffers.scratch.as_entire_binding()),
            ],
        })
    }

    pub fn raster_bind_group(
        &self,
        device: &wgpu::Device,
        buffers: &ClusterBuffers,
        lights: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cluster_raster_bg"),
            layout: &self.raster_layout,
            entries: &[
                entry(0, buffers.view.as_entire_binding()),
                entry(1, lights.as_entire_binding()),
                entry(2, buffers.work_list.as_entire_binding()),
                entry(3, buffers.cells.as_entire_binding()),
                entry(4, buffers.scratch.as_entire_binding()),
                entry(5, buffers.indices.as_entire_binding()),
            ],
        })
    }

    /// Records the whole build.
    ///
    /// `lights` is how many are in the buffer; zero skips everything
    /// after the clear, which leaves every cell empty and every offset
    /// at zero — an unlit scene, said in the only way the shading loop
    /// can read.
    pub fn record(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &ClusterBuffers,
        build_bg: &wgpu::BindGroup,
        raster_bg: &wgpu::BindGroup,
        lights: u32,
        cells: u32,
    ) {
        // Counts accumulate, so they start at zero. The scratch is
        // cleared by the allocation pass instead, where a thread is
        // already visiting every cell.
        encoder.clear_buffer(&buffers.cells, 0, None);
        if lights == 0 {
            return;
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cluster: z-slice"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, build_bg, &[]);
            pass.set_pipeline(&self.z_slice);
            pass.dispatch_workgroups(lights.div_ceil(Z_SLICE_GROUP), 1, 1);
            pass.set_pipeline(&self.finalize);
            pass.dispatch_workgroups(1, 1, 1);
        }

        self.rasterize(encoder, buffers, raster_bg, &self.count, "cluster: count");

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("cluster: allocate"),
                timestamp_writes: None,
            });
            pass.set_bind_group(0, build_bg, &[]);
            pass.set_pipeline(&self.allocate_local);
            pass.dispatch_workgroups(cells.div_ceil(ALLOCATE_BLOCK), 1, 1);
            // One workgroup on purpose: carrying each block's total into
            // the next is sequential, and a second dispatch is what
            // guarantees the first one finished.
            pass.set_pipeline(&self.allocate_global);
            pass.dispatch_workgroups(1, 1, 1);
        }

        self.rasterize(
            encoder,
            buffers,
            raster_bg,
            &self.populate,
            "cluster: populate",
        );
    }

    /// One rasterizer run, drawn from the count the GPU wrote.
    fn rasterize(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        buffers: &ClusterBuffers,
        bind_group: &wgpu::BindGroup,
        pipeline: &wgpu::RenderPipeline,
        label: &str,
    ) {
        let Some((view, ..)) = self.target.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    // Nothing reads it. The attachment exists because a
                    // render pass needs a viewport and a viewport is
                    // what makes a fragment invocation a cell.
                    store: wgpu::StoreOp::Discard,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw_indirect(&buffers.draw, 0);
    }
}

/// What the rasterizer's unused attachment is made of. One byte per
/// cell, discarded every frame.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

fn module(device: &wgpu::Device, label: &str, body: &str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(format!("{COMMON}\n{body}").into()),
    })
}

fn compute(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry_point: &str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry_point),
        layout: Some(layout),
        module,
        entry_point: Some(entry_point),
        compilation_options: Default::default(),
        cache: None,
    })
}

fn raster(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    label: &str,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            // No vertex buffer: six corners from the vertex index.
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module,
            entry_point: Some("fragment_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: TARGET_FORMAT,
                blend: None,
                // The fragment shader's output goes nowhere. Writing it
                // would be bandwidth spent on a texture nothing samples.
                write_mask: wgpu::ColorWrites::empty(),
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Both windings survive: the quad's corners come out of a
            // bounding box, and a light whose box inverts under an odd
            // camera scale would otherwise vanish from the grid.
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn entry(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

fn buffer_entry(
    binding: u32,
    visibility: wgpu::ShaderStages,
    read_only: bool,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn build_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let compute = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cluster_build_bgl"),
        entries: &[
            uniform_entry(0, compute),
            buffer_entry(1, compute, true),
            buffer_entry(2, compute, false),
            buffer_entry(3, compute, false),
            buffer_entry(4, compute, false),
            buffer_entry(5, compute, false),
        ],
    })
}

fn raster_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
    let fragment = wgpu::ShaderStages::FRAGMENT;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("cluster_raster_bgl"),
        entries: &[
            uniform_entry(0, both),
            buffer_entry(1, both, true),
            buffer_entry(2, both, true),
            // 🔴 Fragment only, and not because the vertex stage has no
            // use for them: a writable storage buffer visible to a
            // vertex shader needs `VERTEX_WRITABLE_STORAGE`, which the
            // handheld target is not guaranteed to have. Everything that
            // writes happens per cell, which is a fragment anyway.
            buffer_entry(3, fragment, false),
            buffer_entry(4, fragment, false),
            buffer_entry(5, fragment, false),
        ],
    })
}

/// Every module the passes compile, as the shader compiler sees it:
/// name, and the common declarations already concatenated.
///
/// Exposed so the tests can parse and validate all four without a
/// device. A clustering bug that is a typo in WGSL would otherwise
/// surface as a panic at pipeline creation, on the machine, in the
/// frame — and only for whoever has a GPU that reaches that path.
pub(super) fn shader_sources() -> Vec<(&'static str, String)> {
    vec![
        ("cluster_z_slice", format!("{COMMON}\n{Z_SLICE}")),
        ("cluster_allocate", format!("{COMMON}\n{ALLOCATE}")),
        (
            "cluster_raster_count",
            format!(
                "{COMMON}\n{}",
                RASTER.replace(POPULATE_PLACEHOLDER, "false")
            ),
        ),
        (
            "cluster_raster_populate",
            format!("{COMMON}\n{}", RASTER.replace(POPULATE_PLACEHOLDER, "true")),
        ),
    ]
}

/// The rasterizer's source before substitution, for the test that pins
/// the placeholder is still there to substitute.
pub(super) const RASTER_TEMPLATE: &str = RASTER;
