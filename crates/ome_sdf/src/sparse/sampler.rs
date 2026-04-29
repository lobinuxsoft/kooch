//! Opaque [`SdfSampler`] trait — pluggable SDF source for the sparse
//! pipeline (issue #136 S3 onward).
//!
//! The classify / populate compute passes do not know whether the SDF
//! they are sampling is an analytic primitive, a baked impostor, a
//! procedural function, a voxelised mesh, or an RPN delta tree. They
//! call `sample_sdf(p: vec3<f32>) -> f32`, which is provided by the
//! sampler fragment the pass concatenates ahead of its own shader
//! source.
//!
//! # Bind group convention
//!
//! Sampler bindings live in **`@group(1)`**. The sparse pipeline
//! reserves `@group(0)` for its own outputs (root indices, needs
//! buffers, classify uniform). Implementations declare their bindings
//! starting at `@group(1) @binding(0)` and report them via
//! [`SdfSampler::bind_group_layout_entries`] /
//! [`SdfSampler::bind_group_entries`] using the same binding numbers.
//!
//! # Concrete implementations
//!
//! [`AnalyticSphereSampler`] is the trivial reference implementation —
//! ships an analytic sphere SDF for the classify-pass tests and serves
//! as the worked example for downstream samplers.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Pluggable SDF source. See module-level docs for the bind group
/// convention; implementations are free to use any GPU resources they
/// want as long as the WGSL fragment they emit defines exactly one
/// `fn sample_sdf(p: vec3<f32>) -> f32`.
pub trait SdfSampler {
    /// WGSL fragment defining `fn sample_sdf(p: vec3<f32>) -> f32` plus
    /// any `@group(1)` binding declarations the function reads.
    /// Concatenated ahead of the pass shader source via `format!`.
    fn wgsl_source(&self) -> &str;

    /// Bind group layout entries describing the resources `wgsl_source`
    /// declares. Binding numbers are sampler-local (always within
    /// `@group(1)`); the host wires them as the second bind group on
    /// the pipeline layout.
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

/// WGSL fragment for [`AnalyticSphereSampler`]. Defines
/// `fn sample_sdf` plus the single uniform binding the function reads.
/// `pub` so tests in sibling modules can splice it without round-
/// tripping through the trait.
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
            label: Some("ome_sdf::sparse::analytic_sphere_uniform"),
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
