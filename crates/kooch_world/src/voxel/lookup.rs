//! SDF lookup consumers splice in: [`LOOKUP_BODY_WGSL`] declares no bindings, [`lookup_wgsl`]
//! attaches them (default `@group(2)`). Binds only LOD 0's `root_indices`, which matches every LOD.

use bytemuck::{Pod, Zeroable};
use kooch_core::Aabb;

use super::{ATLAS_TILES_X, ATLAS_TILES_Y, ATLAS_TILES_Z, LOD_LEVELS, ROOT_DIM, SparseGrid};

/// Body only: declares `LookupUniform` and the functions but not the globals, which [`lookup_wgsl`]
/// prepends.
pub const LOOKUP_BODY_WGSL: &str = include_str!("../../shaders/sparse_lookup_body.wgsl");

/// Default `@group` for the lookup globals. See module docs.
pub const LOOKUP_DEFAULT_GROUP: u32 = 2;

/// Default `@binding` for `lookup_root_indices` (canonical, see module
/// docs).
pub const LOOKUP_DEFAULT_ROOT_BINDING: u32 = 0;

/// Default `@binding`s for the four per-LOD `lookup_subgrid_pool_lod*`
/// 3D textures.
pub const LOOKUP_DEFAULT_POOL_BINDINGS: [u32; 4] = [1, 2, 3, 4];

/// Default `@binding` for `lookup_pool_sampler` (filtering sampler).
pub const LOOKUP_DEFAULT_SAMPLER_BINDING: u32 = 5;

/// Default `@binding` for `lookup_chunk_lod_mask` (read-only storage).
pub const LOOKUP_DEFAULT_MASK_BINDING: u32 = 6;

/// Default `@binding` for `lookup_uniform`.
pub const LOOKUP_DEFAULT_UNIFORM_BINDING: u32 = 7;

/// A complete fragment exposing `sparse_sdf_lookup` with the caller's binding slots, plus
/// [`ROOT_DIM`] and [`ATLAS_TILES_X`]/[`ATLAS_TILES_Y`]/[`ATLAS_TILES_Z`] as constants, so
/// consumers do not track `large-root-grid`.
pub fn lookup_wgsl(
    group: u32,
    root_binding: u32,
    pool_bindings: [u32; 4],
    uniform_binding: u32,
    sampler_binding: u32,
    mask_binding: u32,
) -> String {
    let [pool0, pool1, pool2, pool3] = pool_bindings;
    format!(
        "const LOOKUP_ROOT_DIM: u32 = {ROOT_DIM}u;\n\
         const LOOKUP_ATLAS_TILES_X: u32 = {ATLAS_TILES_X}u;\n\
         const LOOKUP_ATLAS_TILES_Y: u32 = {ATLAS_TILES_Y}u;\n\
         const LOOKUP_ATLAS_TILES_Z: u32 = {ATLAS_TILES_Z}u;\n\
         @group({group}) @binding({root_binding}) var<storage, read> lookup_root_indices: array<u32>;\n\
         @group({group}) @binding({pool0}) var lookup_subgrid_pool_lod0: texture_3d<f32>;\n\
         @group({group}) @binding({pool1}) var lookup_subgrid_pool_lod1: texture_3d<f32>;\n\
         @group({group}) @binding({pool2}) var lookup_subgrid_pool_lod2: texture_3d<f32>;\n\
         @group({group}) @binding({pool3}) var lookup_subgrid_pool_lod3: texture_3d<f32>;\n\
         @group({group}) @binding({sampler_binding}) var lookup_pool_sampler: sampler;\n\
         @group({group}) @binding({mask_binding}) var<storage, read> lookup_chunk_lod_mask: LookupChunkLodMask;\n\
         @group({group}) @binding({uniform_binding}) var<uniform> lookup_uniform: LookupUniform;\n\
         {LOOKUP_BODY_WGSL}",
    )
}

/// Host mirror of the WGSL `LookupUniform`. 48 B std140 (three
/// `vec4<f32>`s) — must stay byte-for-byte equal to the struct in
/// `sparse_lookup_body.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct LookupUniformHost {
    bounds_min: [f32; 4],
    bounds_max: [f32; 4],
    cell_size_base: [f32; 4],
}

/// Binding companion to [`lookup_wgsl`]: owns the uniform and builds the layout and bind-group
/// entries over LOD 0's `root_indices`, the four atlases, the sampler and the mask.
pub struct LookupBindings {
    uniform_buffer: wgpu::Buffer,
}

impl LookupBindings {
    /// Allocate the 48 B uniform buffer. Single allocation per
    /// chunk-bound consumer — call [`Self::write`] before each pass
    /// that uses a different `bounds`.
    pub fn new(device: &wgpu::Device) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kooch_world::voxel::lookup::uniform"),
            size: std::mem::size_of::<LookupUniformHost>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { uniform_buffer }
    }

    /// Stages the bounds; LOD choice needs only cell pitch, which the per-LOD factor multiplies
    /// directly.
    pub fn write(&self, queue: &wgpu::Queue, bounds: Aabb) {
        let extent = bounds.max - bounds.min;
        let cell_size = extent / (super::ROOT_DIM as f32);
        let voxel_pitch_lod0 = cell_size / (LOD_LEVELS[0].subgrid_dim as f32);
        // Use the smallest axis's voxel pitch as the scalar
        // `cell_size_base`. The three components are stored in case a
        // future API surface needs the per-axis pitch directly.
        let scalar = voxel_pitch_lod0
            .x
            .min(voxel_pitch_lod0.y)
            .min(voxel_pitch_lod0.z);
        let host = LookupUniformHost {
            bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z, 0.0],
            bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z, 0.0],
            cell_size_base: [scalar, voxel_pitch_lod0.y, voxel_pitch_lod0.z, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&host));
    }

    #[cfg(test)]
    /// Bind group layout entries matching [`lookup_wgsl`]'s prepended
    /// declarations. Visibility is `COMPUTE | FRAGMENT | VERTEX` so a
    /// single layout serves any of the three consumer pipelines.
    pub fn layout_entries(
        root_binding: u32,
        pool_bindings: [u32; 4],
        uniform_binding: u32,
        sampler_binding: u32,
        mask_binding: u32,
    ) -> [wgpu::BindGroupLayoutEntry; 8] {
        let visibility =
            wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX;
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        [
            wgpu::BindGroupLayoutEntry {
                binding: root_binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture_entry(pool_bindings[0]),
            texture_entry(pool_bindings[1]),
            texture_entry(pool_bindings[2]),
            texture_entry(pool_bindings[3]),
            wgpu::BindGroupLayoutEntry {
                binding: sampler_binding,
                visibility,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: mask_binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: uniform_binding,
                visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]
    }

    /// Bind group entries matching the layout above.
    pub fn bind_group_entries<'a>(
        &'a self,
        grid: &'a SparseGrid,
        root_binding: u32,
        pool_bindings: [u32; 4],
        uniform_binding: u32,
        sampler_binding: u32,
        mask_binding: u32,
    ) -> [wgpu::BindGroupEntry<'a>; 8] {
        [
            wgpu::BindGroupEntry {
                binding: root_binding,
                resource: grid.root_indices_buffer(0).as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: pool_bindings[0],
                resource: wgpu::BindingResource::TextureView(grid.subgrid_pool_view(0)),
            },
            wgpu::BindGroupEntry {
                binding: pool_bindings[1],
                resource: wgpu::BindingResource::TextureView(grid.subgrid_pool_view(1)),
            },
            wgpu::BindGroupEntry {
                binding: pool_bindings[2],
                resource: wgpu::BindingResource::TextureView(grid.subgrid_pool_view(2)),
            },
            wgpu::BindGroupEntry {
                binding: pool_bindings[3],
                resource: wgpu::BindingResource::TextureView(grid.subgrid_pool_view(3)),
            },
            wgpu::BindGroupEntry {
                binding: sampler_binding,
                resource: wgpu::BindingResource::Sampler(grid.subgrid_pool_sampler()),
            },
            wgpu::BindGroupEntry {
                binding: mask_binding,
                resource: grid.chunk_lod_mask_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: uniform_binding,
                resource: self.uniform_buffer.as_entire_binding(),
            },
        ]
    }

    /// Direct accessor — useful when the caller wants to assemble the
    /// bind group with non-default ordering.
    pub fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform_buffer
    }
}

#[cfg(test)]
mod tests;
