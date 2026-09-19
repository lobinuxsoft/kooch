//! The lights' bind group layout, and the buffers and dummies a fresh `GpuLights` starts with.

use super::*;

impl GpuLights {
    /// The layout both shading pipelines declare. An associated
    /// function because a pipeline layout has to be built before any
    /// `GpuLights` exists — the same shape `MaterialPool` uses.
    pub fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("inti_lights_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // Visible to both stages because the R64 path shades
                    // in a fragment shader and the R32 fallback in a
                    // compute one, off the same layout.
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<IntiFrame>() as u64,
                        ),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // The shadow atlas in Inti's group; see `TARGET_MAX_BIND_GROUPS`.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        // One layer per cascade, and later per spot
                        // light (#777). The layer is an argument to the
                        // sample call, not a second binding.
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                // Comparison sampler: the depth test runs on the texture unit, bilinear per tap.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
                // Non-comparison sampler for PCSS's blocker search, which needs the depth. The
                // budget is on groups, not bindings — Bevy does the same. Non-filtering, since wgpu
                // forbids filtering with `Depth`.
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                // The point lights' cube array (#778): a binding in this group, reusing both
                // samplers — a sampler is not bound to a texture.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        // Six layers per light, the light index chosen by
                        // the shader and the face by the direction.
                        view_dimension: wgpu::TextureViewDimension::CubeArray,
                        multisampled: false,
                    },
                    count: None,
                },
                // The froxel grid (#780): cell records, then the index list.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Virtual shadow maps (#866): uniform, two arrays and the atlas. 🔴 The atlas is
                // read with `textureLoad`: a filter cannot stop at a page border.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // Binding 9 held the page hash's key array and is
                // retired: the flat table is indexed by the page id.
                wgpu::BindGroupLayoutEntry {
                    binding: 10,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 11,
                    visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        // One layer per camera. See `page_place`.
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        })
    }

    pub fn new(device: &wgpu::Device) -> Self {
        let layout = Self::bind_group_layout(device);
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("inti_frame_ubo"),
            contents: bytemuck::bytes_of(&IntiFrame::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let light_buffer = create_light_buffer(device, INITIAL_CAPACITY);
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("inti_shadow_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            // Reversed-Z: an occluder is nearer the light and therefore
            // GREATER. The comparison has to match, or every shadow
            // inverts and the lit side goes dark.
            compare: Some(wgpu::CompareFunction::Greater),
            // Clamped: a tap that falls off a cascade should read as the
            // edge rather than wrapping into the neighbouring quadrant,
            // which would be a shadow from the wrong distance.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let shadow_point_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("inti_shadow_point_sampler"),
            // Nearest on both, because the layout declares this
            // non-filtering and wgpu checks that the sampler agrees.
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let dummy_shadow = create_dummy_shadow(device);
        let dummy_cubes = create_dummy_cubes(device);
        let dummy_page_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("inti_dummy_pages"),
            // Large enough for the page uniform when it stands in for
            // it, and harmless as an empty table.
            size: 256,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let dummy_page_atlas = create_dummy_page_atlas(device);
        let clusters = GpuClusters::new(device);
        let bind_group = create_bind_group(
            device,
            &layout,
            &frame_buffer,
            &light_buffer,
            &dummy_shadow,
            &shadow_sampler,
            &shadow_point_sampler,
            &dummy_cubes,
            &clusters,
            PageBinding {
                uniform: &dummy_page_buffer,
                uniform_span: (0, 0),
                slots: &dummy_page_buffer,
                atlas: &dummy_page_atlas,
            },
        );
        Self {
            frame_buffer,
            light_buffer,
            bind_group,
            layout,
            shadow_sampler,
            shadow_point_sampler,
            dummy_shadow,
            dummy_cubes,
            page_uniform: None,
            page_uniform_span: (0, 0),
            page_slots: None,
            page_atlas: None,
            dummy_page_buffer,
            dummy_page_atlas,
            shadow_atlas: None,
            shadow_cubes: None,
            clusters,
            capacity: INITIAL_CAPACITY,
            light_count: 0,
            uploaded: Vec::new(),
        }
    }
}
