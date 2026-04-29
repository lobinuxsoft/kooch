//! Reusable sparse SDF lookup — a pure WGSL function consumers
//! splice into their own pipelines (raymarcher, Edit Baker #309,
//! debug visualisers).
//!
//! The shader source ([`LOOKUP_BODY_WGSL`]) deliberately does not
//! declare `@group/@binding` attributes — those slots are caller
//! responsibility. The Rust helper [`lookup_wgsl`] returns a finished
//! shader fragment with the three required globals
//! (`lookup_root_indices`, `lookup_subgrid_pool`, `lookup_uniform`)
//! attached at the requested binding layout.
//!
//! # Default layout — `(group = 2, root = 0, pool = 1, uniform = 2)`
//!
//! Tied to the contract recommended for the Edit Baker pipeline
//! (#309). `@group(0)` is reserved for the producer's own writeable
//! resources and `@group(1)` for samplers; the lookup globals live in
//! `@group(2)` as a self-contained read-only view of one chunk's
//! sparse SDF. Consumers with conflicting groups override at the
//! [`lookup_wgsl`] call site.
//!
//! # Trilinear filter — clamped, C0 boundary
//!
//! The filter clamps the upper sample corner to the current subgrid's
//! interior (`p1 = min(p0 + 1, 15)`). Samples that fall on a subgrid
//! boundary therefore see no contribution from the neighbour — the
//! seam is C0, not C1. Intentional in S5 to keep the lookup
//! self-contained; S6 (skirt voxels) or S7 (Vulkan hardware sampler
//! over a virtual texture) is where the seam continuity is fixed.
//! See the issue thread for the follow-up.

use bytemuck::{Pod, Zeroable};
use ome_bvh::Aabb;

use super::SparseGrid;

/// WGSL source — body only. Declares `LookupUniform`,
/// `sparse_sdf_far_value`, and `sparse_sdf_lookup`, but not the three
/// globals they read; [`lookup_wgsl`] prepends the `var<...>` decls
/// with the caller's binding slots.
pub const LOOKUP_BODY_WGSL: &str = include_str!("../../shaders/sparse_lookup_body.wgsl");

/// Default `@group` for the three lookup globals. See module docs.
pub const LOOKUP_DEFAULT_GROUP: u32 = 2;

/// Default `@binding` for `lookup_root_indices`.
pub const LOOKUP_DEFAULT_ROOT_BINDING: u32 = 0;

/// Default `@binding` for `lookup_subgrid_pool`.
pub const LOOKUP_DEFAULT_POOL_BINDING: u32 = 1;

/// Default `@binding` for `lookup_uniform`.
pub const LOOKUP_DEFAULT_UNIFORM_BINDING: u32 = 2;

/// Build a complete WGSL fragment exposing `sparse_sdf_lookup`,
/// concatenated with the binding declarations the caller's pipeline
/// layout specifies. Splice into the consumer's shader source ahead
/// of any function that calls `sparse_sdf_lookup`.
///
/// All three globals must sit in the same `@group`. Splitting them
/// across groups is intentionally unsupported — every real consumer
/// so far (Edit Baker, raymarcher, debug viz) binds them as one
/// per-chunk read-only view, and the constraint keeps the helper
/// signature compact. Filed as a follow-up if a real need surfaces.
pub fn lookup_wgsl(
    group: u32,
    root_binding: u32,
    pool_binding: u32,
    uniform_binding: u32,
) -> String {
    format!(
        "@group({group}) @binding({root_binding}) var<storage, read> lookup_root_indices: array<u32>;\n\
         @group({group}) @binding({pool_binding}) var<storage, read> lookup_subgrid_pool: array<f32>;\n\
         @group({group}) @binding({uniform_binding}) var<uniform> lookup_uniform: LookupUniform;\n\
         {LOOKUP_BODY_WGSL}",
    )
}

/// Host mirror of the WGSL `LookupUniform`. 32 B std140 (two
/// `vec4<f32>`s) — must stay byte-for-byte equal to the struct in
/// `sparse_lookup_body.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct LookupUniformHost {
    bounds_min: [f32; 4],
    bounds_max: [f32; 4],
}

/// Minimal binding-side companion to [`lookup_wgsl`]. Owns the
/// uniform buffer, exposes the layout entries the caller's pipeline
/// layout needs, and produces the bind-group entries pointing at
/// `(grid.root_indices, grid.subgrid_pool, self.uniform)`.
pub struct LookupBindings {
    uniform_buffer: wgpu::Buffer,
}

impl LookupBindings {
    /// Allocate the 32 B uniform buffer. Single allocation per
    /// chunk-bound consumer — call [`Self::write`] before each pass
    /// that uses a different `bounds`.
    pub fn new(device: &wgpu::Device) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_sdf::sparse::lookup::uniform"),
            size: std::mem::size_of::<LookupUniformHost>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { uniform_buffer }
    }

    /// Stage the chunk bounds into the uniform buffer. Sequenced ahead
    /// of any encoder commands the caller submits in the same queue
    /// submission (wgpu serialises queue writes before commands).
    pub fn write(&self, queue: &wgpu::Queue, bounds: Aabb) {
        let host = LookupUniformHost {
            bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z, 0.0],
            bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&host));
    }

    /// Bind group layout entries matching [`lookup_wgsl`]'s prepended
    /// declarations. Visibility is `COMPUTE | FRAGMENT | VERTEX` so a
    /// single layout serves any of the three consumer pipelines
    /// without a re-bind dance — the lookup is read-only and stage
    /// fan-out costs nothing in practice.
    pub fn layout_entries(
        root_binding: u32,
        pool_binding: u32,
        uniform_binding: u32,
    ) -> [wgpu::BindGroupLayoutEntry; 3] {
        let visibility = wgpu::ShaderStages::COMPUTE
            | wgpu::ShaderStages::FRAGMENT
            | wgpu::ShaderStages::VERTEX;
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
            wgpu::BindGroupLayoutEntry {
                binding: pool_binding,
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

    /// Bind group entries matching the layout above. Lifetime ties
    /// the returned entries to `&self` and `grid`, so the caller may
    /// build the bind group inline.
    pub fn bind_group_entries<'a>(
        &'a self,
        grid: &'a SparseGrid,
        root_binding: u32,
        pool_binding: u32,
        uniform_binding: u32,
    ) -> [wgpu::BindGroupEntry<'a>; 3] {
        [
            wgpu::BindGroupEntry {
                binding: root_binding,
                resource: grid.root_indices_buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: pool_binding,
                resource: grid.subgrid_pool_buffer().as_entire_binding(),
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
