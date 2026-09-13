//! [`SdfSampler`]: the SDF source the passes call as `sample_sdf(p) -> f32`, blind to what is
//! behind it. Its bindings live in `@group(1)`; `@group(0)` is the pipeline's.
//! [`AnalyticSphereSampler`] is the reference implementation.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Any GPU resources, provided the WGSL fragment defines exactly one `fn sample_sdf(p: vec3<f32>)
/// -> f32`.
pub trait SdfSampler {
    /// WGSL fragment defining `fn sample_sdf(p: vec3<f32>) -> f32` plus
    /// any `@group(1)` binding declarations the function reads.
    /// Concatenated ahead of the pass shader source via `format!`.
    fn wgsl_source(&self) -> &str;

    /// Layout entries for what `wgsl_source` declares, numbered within `@group(1)`.
    fn bind_group_layout_entries(&self) -> Vec<wgpu::BindGroupLayoutEntry>;

    /// Bind group entries matching the layout above. Returns `'a`-
    /// borrowed resources owned by `&self`, so the resulting bind
    /// group lifetime is tied to the sampler.
    fn bind_group_entries(&self) -> Vec<wgpu::BindGroupEntry<'_>>;
}

/// Uniform mirror of the WGSL `AnalyticSphere` struct in
/// [`ANALYTIC_SPHERE_WGSL`]. `xyz` is the centre, `w` is the radius.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct AnalyticSphereUniform {
    center_radius: [f32; 4],
}

/// WGSL for [`AnalyticSphereSampler`]; `pub` so sibling tests splice it without the trait.
pub const ANALYTIC_SPHERE_WGSL: &str = r#"
struct AnalyticSphere {
    center_radius: vec4<f32>,
}

@group(1) @binding(0) var<uniform> analytic_sphere: AnalyticSphere;

fn sample_sdf(p: vec3<f32>) -> f32 {
    return length(p - analytic_sphere.center_radius.xyz) - analytic_sphere.center_radius.w;
}
"#;

/// Reference [`SdfSampler`] backed by an analytic sphere
/// `length(p - center) - radius`. Trivially Lipschitz, used by the
/// classify-pass tests as a CPU-comparable ground truth.
pub struct AnalyticSphereSampler {
    center: Vec3,
    radius: f32,
    uniform_buffer: wgpu::Buffer,
}

impl AnalyticSphereSampler {
    /// Allocate the uniform buffer and seed it with `(center, radius)`.
    /// `mapped_at_creation` keeps the upload off the queue staging
    /// belt — this runs once per sampler instance, not per frame.
    pub fn new(device: &wgpu::Device, center: Vec3, radius: f32) -> Self {
        let uniform = AnalyticSphereUniform {
            center_radius: [center.x, center.y, center.z, radius],
        };
        let bytes = bytemuck::bytes_of(&uniform);
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kooch_world::voxel::analytic_sphere_uniform"),
            size: bytes.len() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        uniform_buffer
            .slice(..)
            .get_mapped_range_mut()
            .copy_from_slice(bytes);
        uniform_buffer.unmap();
        Self {
            center,
            radius,
            uniform_buffer,
        }
    }

    pub fn center(&self) -> Vec3 {
        self.center
    }

    pub fn radius(&self) -> f32 {
        self.radius
    }

    #[cfg(test)]
    /// CPU mirror of the WGSL `sample_sdf` — used by tests to compare
    /// classify-pass output against a brute-force ground truth.
    pub fn sample_cpu(&self, p: Vec3) -> f32 {
        (p - self.center).length() - self.radius
    }
}

impl SdfSampler for AnalyticSphereSampler {
    fn wgsl_source(&self) -> &str {
        ANALYTIC_SPHERE_WGSL
    }

    fn bind_group_layout_entries(&self) -> Vec<wgpu::BindGroupLayoutEntry> {
        vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }]
    }

    fn bind_group_entries(&self) -> Vec<wgpu::BindGroupEntry<'_>> {
        vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: self.uniform_buffer.as_entire_binding(),
        }]
    }
}
