//! Building the rasterizer: its shaders, layouts, pipelines and buffers.

use super::*;

impl PageRasterizer {
    pub fn new(
        device: &wgpu::Device,
        meshlet_bgl: &wgpu::BindGroupLayout,
        config: PageConfig,
        clipmap: ClipmapConfig,
        pool: PoolConfig,
        max_triangles_per_meshlet: u32,
    ) -> Self {
        let atlas = atlas_texture(device, config, pool);
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layers = (0..atlas.depth_or_array_layers())
            .map(|layer| {
                atlas.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("shadow_page_atlas_layer"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: layer,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let levels = clipmap.levels;
        // Sun buckets plus one bucket per lamp; every per-bucket buffer
        // is sized for both halves.
        let buckets = levels + LAMP_CULLS;

        let _module = |label: &str, body: &str| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(format!("{TABLE}\n{body}").into()),
            })
        };
        let compact_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_compact"),
            source: wgpu::ShaderSource::Wgsl(
                format!("{CLUSTER_COMMON}\n{TABLE}\n{COMPACT}").into(),
            ),
        });
        // The expansion reaches for `ClusterLight` too: a lamp's page is
        // a frustum from the light's own position and range, not a slab.
        let expand_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_expand"),
            source: wgpu::ShaderSource::Wgsl(
                format!(
                    "{CLUSTER_COMMON}\n{TABLE}\n{}\n{EXPAND}",
                    super::pyramid::OVERLAP
                )
                .into(),
            ),
        });
        // The depth pass builds a lamp's frustum from the light record.
        let clipped = device.features().contains(wgpu::Features::CLIP_DISTANCES);
        let enable = if clipped {
            "enable clip_distances;\n"
        } else {
            ""
        };
        let tail = if clipped { DEPTH_CLIPPED } else { "" };
        let depth_source = format!("{enable}{CLUSTER_COMMON}\n{TABLE}\n{DEPTH}\n{tail}");
        let depth_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("page_depth"),
            source: wgpu::ShaderSource::Wgsl(depth_source.as_str().into()),
        });

        let compact_bgl = compact_layout(device);
        let compact_layout_pipeline =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_compact_layout"),
                bind_group_layouts: &[Some(&compact_bgl)],
                immediate_size: 0,
            });
        let compute = |entry: &str, module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(layout),
                module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let compact = compute("cs_compact", &compact_module, &compact_layout_pipeline);
        let expand_args_pass = compute("cs_expand_args", &compact_module, &compact_layout_pipeline);
        let lod_offsets = compute("cs_lod_offsets", &compact_module, &compact_layout_pipeline);
        let draw_args_pass = compute("cs_draw_args", &compact_module, &compact_layout_pipeline);
        let invalidate_bgl = invalidate_layout(device);
        let invalidate_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_invalidate_layout"),
                bind_group_layouts: &[Some(&invalidate_bgl)],
                immediate_size: 0,
            });
        let invalidate = compute(
            "cs_invalidate",
            &compact_module,
            &invalidate_pipeline_layout,
        );

        let expand_bgl = expand_layout(device);
        let storage_bgl = storage_layout(
            device,
            wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::VERTEX,
        );
        let expand_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_expand_layout"),
                // 🔴 Its OWN one-buffer layout for the descriptors, not the meshlet pool's five.
                // `max_storage_buffers_ _per_shader_stage` is 8 by default and the pool alone would
                // spend five of them on four buffers this pass never reads.
                bind_group_layouts: &[
                    Some(&expand_bgl),
                    Some(&storage_bgl),
                    Some(&storage_bgl),
                    Some(&storage_bgl),
                ],
                immediate_size: 0,
            });
        let expand = compute("cs_expand", &expand_module, &expand_pipeline_layout);

        let depth_bgl = depth_layout(device);
        let depth_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_depth_layout"),
                bind_group_layouts: &[Some(&depth_bgl), Some(meshlet_bgl), Some(&storage_bgl)],
                immediate_size: 0,
            });
        let depth = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("page_depth"),
            layout: Some(&depth_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &depth_module,
                entry_point: Some(if clipped {
                    "vs_page_clipped"
                } else {
                    "vs_page"
                }),
                buffers: &[],
                compilation_options: Default::default(),
            },
            // 🔴 A fragment stage where `shadow_depth` has none, and it is not an oversight: it is
            // the per-page scissor the hardware cannot give per instance. See `page_depth.wgsl`.
            fragment: (!clipped).then(|| wgpu::FragmentState {
                module: &depth_module,
                entry_point: Some("fs_page"),
                targets: &[],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: PAGE_FRONT_FACE,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: PAGE_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // Reversed-Z, like every other depth test in the engine.
                depth_compare: Some(wgpu::CompareFunction::Greater),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let depth_alpha = alpha::depth_alpha(
            device,
            &depth_source,
            [&depth_bgl, meshlet_bgl, &storage_bgl],
            clipped,
        );

        let clear_bgl = clear_layout(device);
        let clear_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("page_clear_layout"),
                bind_group_layouts: &[Some(&clear_bgl)],
                immediate_size: 0,
            });
        let page_clear = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("page_clear"),
            layout: Some(&clear_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &depth_module,
                entry_point: Some("vs_page_clear"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            // No fragment and no scissor needed: the quad's corners ARE
            // the page's rect, so nothing rasterises past it.
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: PAGE_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                // A clear, as a draw: it always wins.
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let align = device.limits().min_uniform_buffer_offset_alignment as u64;
        let level_stride = align.max(std::mem::size_of::<ExpandLevel>() as u64);
        // 🔴 Rounded UP to a multiple, not `max`. A dynamic offset has to be a multiple of
        // `min_uniform_buffer_offset_alignment`, and `max` only guarantees it when the struct is
        // smaller than the alignment.
        let uniform_stride = (std::mem::size_of::<RasterUniform>() as u64)
            .div_ceil(align)
            .max(1)
            * align;
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST;

        Self {
            atlas,
            atlas_view,
            layers,
            uniform: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_uniform"),
                size: uniform_stride * atlas_layers(pool) as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            uniform_stride,
            page_list: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_list"),
                // Four words per listing: page, slot, bound (#940), spare.
                size: bucket(pool) as u64 * buckets as u64 * 16,
                // 🔴 COPY_SRC because `page_list_buffer` is public and the
                // only reason to expose a GPU buffer is to read it back.
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            counts: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_counts"),
                size: count_slots(buckets) as u64 * 4,
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            expand_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_expand_args"),
                size: buckets as u64 * 12,
                usage: storage | wgpu::BufferUsages::INDIRECT,
                mapped_at_creation: false,
            }),
            draw_args: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_draw_args"),
                // Two draws: the pairs, then one quad per dirty page.
                size: 32,
                usage: storage | wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            pairs: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_pairs"),
                size: PAIR_CAPACITY as u64 * 16,
                // COPY_SRC so a test can read the pairs back: the one claim #1022 makes is that the
                // two shapes emit the SAME ones, and that is only checkable by comparing the lists.
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            visible_counts: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_visible_counts"),
                size: buckets as u64 * 4,
                // 🔴 COPY_SRC: the survivor counts ride home in the same
                // readback as the page counts, so the expansion's cost
                // can be read as the product it is. See `count_slots`.
                usage: storage | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            levels: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_levels"),
                size: level_stride * buckets as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            level_stride,
            gens: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_gens"),
                size: atlas_layers(pool) as u64 * buckets as u64 * 4,
                usage: storage,
                mapped_at_creation: false,
            }),
            dirty: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_dirty"),
                // A header word, then at most one slot per page a view
                // owns — the most one compaction can list.
                size: (1 + pool.slots() as u64) * 4,
                usage: storage,
                mapped_at_creation: false,
            }),
            moved: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("page_raster_moved"),
                size: moved_bytes(MOVED_CAPACITY),
                usage: storage,
                mapped_at_creation: false,
            }),
            moved_capacity: MOVED_CAPACITY,
            scene_gen: 0,
            scene_epoch: None,
            moved_frame: None,
            flooded: false,
            compact_bgl,
            compact,
            expand_args_pass,
            lod_offsets,
            draw_args_pass,
            expand_bgl,
            storage_bgl,
            expand,
            depth_bgl,
            depth,
            depth_alpha,
            invalidate,
            invalidate_bgl,
            page_clear,
            clear_bgl,
            frame: 0,
            softness: 1,
            // What `inti_pbr.wgsl` held as constants before the
            // settings could reach it, so a project with no settings
            // file renders exactly as it did.
            bias: [1.8, 0.02, 0.0, 4.0],
            march: false,
            geometry: false,
            pyramid: PagePyramid::new(device, config, clipmap),
            triangles: max_triangles_per_meshlet.max(1),
            two_level: crate::meshlet::MeshletLodSettings::default().two_level,
            culls: (0..levels)
                .map(|_| MeshletCull::new(device, 1, max_triangles_per_meshlet))
                .collect(),
            lamp_cull: super::lamp_cull::LampCull::new(device),
            lamp_frame: None,
            bound: None,
            readback: RasterReadback::new(device, count_slots(buckets)),
            config,
            clipmap,
            pool,
        }
    }
}
