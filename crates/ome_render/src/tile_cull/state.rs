//! `TileCullState` — wgpu-side compute pre-pass that emits per-tile
//! ray bounds against the coarsest GDF cascade.
//!
//! The state owns the compute pipeline, the per-frame uniform buffer
//! (16 B), the persistent `tile_ray_bounds` SSBO, and the compute-side
//! bind group that ties them together with the existing camera +
//! `GdfState` resources. The fragment shader's group 2 also reads the
//! same SSBO + UBO; that bind group lives in `RayMarchRenderer`'s
//! scene wiring so this module doesn't need to know about it.
//!
//! The SSBO grows on demand: when the viewport tile-count increases,
//! the buffer is reallocated and the compute bind group rebuilt.
//! Shrinks keep the existing capacity (no churn on transient resizes).
//! `STORAGE | COPY_SRC` so tests can read it back via a staging buffer.

use bytemuck::{Pod, Zeroable};

use super::uniforms::{TileBounds, TileCullUniforms};
use crate::gdf::GdfState;

const COMPUTE_SHADER_SOURCE: &str = include_str!("../../shaders/tile_cull.wgsl");

/// Minimum SSBO entries kept allocated. Avoids reallocating on every
/// resize for tiny viewports (tests, hidden windows).
const MIN_TILE_CAPACITY: u32 = 64; // 1 KB floor.

/// Compute pre-pass state.
pub struct TileCullState {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    uniforms_buffer: wgpu::Buffer,
    tile_bounds_buffer: wgpu::Buffer,
    tile_bounds_capacity: u32,
    bind_group: wgpu::BindGroup,
    last_uniforms: TileCullUniforms,
}

impl TileCullState {
    /// Build the compute pipeline, allocate the floor-sized SSBO, and
    /// wire the compute bind group to the supplied camera + GDF
    /// resources. Both `camera_buffer` and `gdf` are expected to be
    /// stable for the renderer's lifetime — `TileCullState` stores no
    /// references, only the resulting bind group.
    pub fn new(
        device: &wgpu::Device,
        camera_buffer: &wgpu::Buffer,
        gdf: &GdfState,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_render::tile_cull::shader"),
            source: wgpu::ShaderSource::Wgsl(COMPUTE_SHADER_SOURCE.into()),
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_render::tile_cull::bgl"),
            entries: &compute_bgl_entries(),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_render::tile_cull::pipeline_layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_render::tile_cull::pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("cs_tile_cull"),
            compilation_options: Default::default(),
            cache: None,
        });

        let last_uniforms = TileCullUniforms::default();
        let uniforms_buffer = create_uniforms_buffer(device, &last_uniforms);
        let tile_bounds_buffer = create_tile_bounds_buffer(device, MIN_TILE_CAPACITY);
        let bind_group = create_bind_group(
            device,
            &bgl,
            camera_buffer,
            &uniforms_buffer,
            gdf.frag_uniforms_buffer(),
            gdf.cascade_view(5),
            gdf.sampler(),
            &tile_bounds_buffer,
        );

        Self {
            pipeline,
            bgl,
            uniforms_buffer,
            tile_bounds_buffer,
            tile_bounds_capacity: MIN_TILE_CAPACITY,
            bind_group,
            last_uniforms,
        }
    }

    /// Run the tile cull compute pass for `(width, height)` viewport.
    ///
    /// Reallocates the SSBO + rebuilds the compute bind group when the
    /// tile count grows past `tile_bounds_capacity`. Rebinds the
    /// fragment-side bind group is **not** triggered here — the caller
    /// (`RayMarchRenderer`) re-reads `tile_bounds_buffer()` if it
    /// needs to mirror the rebuild on its own side.
    ///
    /// Returns `true` when the SSBO was reallocated this call. Caller
    /// uses the flag to know whether the fragment-side scene bind group
    /// needs a rebuild as well.
    pub fn dispatch(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        camera_buffer: &wgpu::Buffer,
        gdf: &GdfState,
        viewport_width: u32,
        viewport_height: u32,
    ) -> bool {
        let new_uniforms = TileCullUniforms::for_viewport(
            viewport_width.max(1),
            viewport_height.max(1),
        );
        let needed = new_uniforms.tile_count_total().max(MIN_TILE_CAPACITY);
        let mut realloc = false;
        if needed > self.tile_bounds_capacity {
            self.tile_bounds_buffer = create_tile_bounds_buffer(device, needed);
            self.tile_bounds_capacity = needed;
            self.bind_group = create_bind_group(
                device,
                &self.bgl,
                camera_buffer,
                &self.uniforms_buffer,
                gdf.frag_uniforms_buffer(),
                gdf.cascade_view(5),
                gdf.sampler(),
                &self.tile_bounds_buffer,
            );
            realloc = true;
        }
        if new_uniforms != self.last_uniforms {
            queue.write_buffer(
                &self.uniforms_buffer,
                0,
                bytemuck::bytes_of(&new_uniforms),
            );
            self.last_uniforms = new_uniforms;
        }

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_render::tile_cull::pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.dispatch_workgroups(
            new_uniforms.tile_count[0],
            new_uniforms.tile_count[1],
            1,
        );
        realloc
    }

    /// SSBO storing one [`TileBounds`] entry per tile in the active
    /// viewport. Fragment shader reads it as `read_only` storage.
    pub fn tile_bounds_buffer(&self) -> &wgpu::Buffer {
        &self.tile_bounds_buffer
    }

    /// Per-frame UBO. Fragment shader reads `tile_count` to map a pixel
    /// to its tile index in the SSBO.
    pub fn uniforms_buffer(&self) -> &wgpu::Buffer {
        &self.uniforms_buffer
    }

    /// Most recent uniforms written. Tests use the `tile_count_total`
    /// to size readback staging buffers.
    pub fn last_uniforms(&self) -> TileCullUniforms {
        self.last_uniforms
    }

    /// Current SSBO capacity in tile entries.
    pub fn tile_bounds_capacity(&self) -> u32 {
        self.tile_bounds_capacity
    }
}

fn create_uniforms_buffer(device: &wgpu::Device, init: &TileCullUniforms) -> wgpu::Buffer {
    use wgpu::util::DeviceExt;
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ome_render::tile_cull::uniforms_buffer"),
        contents: bytemuck::bytes_of(init),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

fn create_tile_bounds_buffer(device: &wgpu::Device, tile_count: u32) -> wgpu::Buffer {
    let size = u64::from(tile_count) * std::mem::size_of::<TileBounds>() as u64;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ome_render::tile_cull::tile_bounds_buffer"),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    camera_buffer: &wgpu::Buffer,
    uniforms_buffer: &wgpu::Buffer,
    gdf_uniforms_buffer: &wgpu::Buffer,
    cascade_5_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    tile_bounds_buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ome_render::tile_cull::bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: camera_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: uniforms_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: gdf_uniforms_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(cascade_5_view) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 5, resource: tile_bounds_buffer.as_entire_binding() },
        ],
    })
}

fn compute_bgl_entries() -> [wgpu::BindGroupLayoutEntry; 6] {
    let uniform = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let storage_rw = wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Storage { read_only: false },
        has_dynamic_offset: false,
        min_binding_size: None,
    };
    let cascade_texture = wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D3,
        multisampled: false,
    };
    let cascade_sampler = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
    let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty,
        count: None,
    };
    [
        entry(0, uniform),          // camera
        entry(1, uniform),          // tile_cull_u
        entry(2, uniform),          // gdf_uniforms
        entry(3, cascade_texture),  // cascade 5 view
        entry(4, cascade_sampler),  // gdf sampler
        entry(5, storage_rw),       // tile_ray_bounds
    ]
}

// Pod conformance pinned by `uniforms.rs::tests::tile_cull_uniforms_layout`,
// but require the trait bound here so a future struct rename can't
// silently drop `Pod`/`Zeroable`.
const _: fn() = || {
    fn assert_pod<T: Pod + Zeroable>() {}
    assert_pod::<TileCullUniforms>();
    assert_pod::<TileBounds>();
};
