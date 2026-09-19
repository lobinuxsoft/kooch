//! The page passes' bind group layouts and entries.

pub(in crate::shadow::pages) fn entry(
    binding: u32,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

pub(in crate::shadow::pages) fn buffer_entry(
    binding: u32,
    read_only: bool,
    visibility: wgpu::ShaderStages,
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

pub(in crate::shadow::pages) fn uniform_entry(
    binding: u32,
    dynamic: bool,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: dynamic,
            min_binding_size: None,
        },
        count: None,
    }
}

pub(super) fn storage_layout(
    device: &wgpu::Device,
    visibility: wgpu::ShaderStages,
) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_raster_storage_bgl"),
        entries: &[buffer_entry(0, true, visibility)],
    })
}

pub(super) fn compact_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let c = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_compact_bgl"),
        entries: &[
            // Dynamic, so one bind group serves every camera: its slice
            // of the uniform travels as an offset instead of as a
            // second allocation.
            uniform_entry(0, true, c),
            // Binding 1 held the hash table's keys and is retired: the flat table's entry index IS
            // the page id. 🔴 Writable now: the compaction records each page's place in `page_list`
            // back into its table entry, which is the only pass that knows both. See `PAGE_CELL`.
            buffer_entry(2, false, c),
            buffer_entry(3, false, c),
            buffer_entry(4, false, c),
            buffer_entry(5, false, c),
            buffer_entry(6, true, c),
            buffer_entry(7, false, c),
            // The generations the cache gate compares stamps against, and the dirty list the
            // per-page clear draws from — the eighth storage buffer, which is the whole downlevel
            // budget again.
            buffer_entry(8, true, c),
            buffer_entry(9, false, c),
        ],
    })
}

/// `cs_invalidate`'s own layout: the table, the moved spheres and the
/// lights — none of which the other compact entries touch.
pub(super) fn invalidate_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let c = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_invalidate_bgl"),
        entries: &[
            uniform_entry(0, true, c),
            buffer_entry(2, false, c),
            buffer_entry(10, true, c),
            buffer_entry(11, true, c),
        ],
    })
}

/// The per-page clear's layout: the uniform for the atlas arithmetic
/// and the dirty list naming the slots to wipe.
pub(super) fn clear_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let v = wgpu::ShaderStages::VERTEX;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_clear_bgl"),
        entries: &[uniform_entry(0, true, v), buffer_entry(3, true, v)],
    })
}

pub(super) fn expand_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let c = wgpu::ShaderStages::COMPUTE;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_expand_bgl"),
        entries: &[
            uniform_entry(0, true, c),
            buffer_entry(1, true, c),
            buffer_entry(2, false, c),
            buffer_entry(3, false, c),
            buffer_entry(4, true, c),
            uniform_entry(5, true, c),
            // 🔴 Here rather than in a group of its own: `max_bind_groups` is FOUR and this pass
            // already binds four. It is also the eighth storage buffer of the stage, which is the
            // entire downlevel budget.
            buffer_entry(6, true, c),
            // Which is why the pyramid is a TEXTURE. There is no ninth
            // storage buffer to give it, and textures are a separate
            // budget — the same constraint that decided Unreal's shape.
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: c,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
        ],
    })
}

pub(super) fn depth_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    let v = wgpu::ShaderStages::VERTEX;
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("page_depth_bgl"),
        entries: &[
            uniform_entry(0, true, v),
            buffer_entry(1, true, v),
            buffer_entry(2, true, v),
        ],
    })
}
