//! Classify pass — flag root cells whose centres fall within one
//! cell-diagonal of the sampled SDF surface (single-sample Lipschitz
//! cone test).
//!
//! GPU-driven: 1 SDF eval per root cell, 4096 evals per chunk, 64
//! workgroups × 64 threads. Output is an indirect-ready compaction so
//! the allocate pass (S4) can `dispatch_workgroups_indirect` over only
//! the marked cells without a CPU readback in the hot loop.
//!
//! Two compute pipelines compose into one [`record`] call:
//!
//! 1. `classify_main` (concatenated with the [`SdfSampler`] WGSL
//!    fragment) writes `(needs_indices, needs_count)`.
//! 2. `finalize_main` (sampler-independent) reads `needs_count` and
//!    writes `[ceil_div(n, 64), 1, 1]` into the indirect-args buffer.
//!
//! The split keeps the sampler concatenation surface confined to the
//! classify shader; finalize never sees the sampler fragment.
//!
//! # Indirect-args shape — tradeoff
//!
//! Two viable layouts for the dispatch arguments:
//!
//! - **(A) Two buffers + finalize pass** (chosen): a 4-byte
//!   `needs_count` plus a 12-byte `[x, y, z]` indirect-args triple,
//!   linked by a 1-thread compute pass that derives `x = ⌈n / 64⌉`.
//!   Cost: one extra single-thread dispatch per chunk classification.
//!   Wins because (i) consumers that only need the count
//!   (diagnostics, tests) read a plain `u32`, (ii) downstream passes
//!   can pick whichever workgroup size they want — finalize divides
//!   by their constant, not the classify constant.
//! - **(B) Single buffer with `[count, 1, 1]` shape**: classify
//!   `atomicAdd`s straight into `args.x`. No finalize pass. But then
//!   the consumer's workgroup size is locked to 1 thread per workgroup
//!   (or it has to redo the divide on the GPU anyway), so the
//!   apparent simplicity bleeds into S4. Rejected.

use bytemuck::{Pod, Zeroable};

use super::{ROOT_CELLS, SparseGrid};

/// WGSL source of the classify pass body — `classify_main` plus the
/// `@group(0)` binding declarations. Concatenated with the sampler
/// fragment by [`ClassifyPass::new`].
pub const CLASSIFY_WGSL: &str = include_str!("../../shaders/sparse_classify.wgsl");

/// WGSL source of the finalize pass — `finalize_main` deriving the
/// indirect-args triple from `needs_count`. Standalone (no sampler
/// concatenation).
pub const CLASSIFY_FINALIZE_WGSL: &str =
    include_str!("../../shaders/sparse_classify_finalize.wgsl");

/// Workgroup size matching the `@workgroup_size(64)` annotation in
/// `sparse_classify.wgsl`. Kept as a Rust constant so the dispatch
/// math can't drift from the shader.
pub const CLASSIFY_WORKGROUP_SIZE: u32 = 64;

/// Default cone-test margin. `1.0` is the exact Lipschitz cone radius
/// for unit-Lipschitz SDFs (`|sdf(center)| < cell_diagonal` ↔ the
/// surface intersects the cell). Non-Lipschitz samplers (smooth blends
/// of analytic primitives, certain raymarch ESL variants) may produce
/// false negatives at this margin — bump it via the `margin` argument
/// to [`ClassifyPass::record`] when calibrating.
pub const DEFAULT_MARGIN: f32 = 1.0;

/// Uniform mirror — must match the WGSL `ClassifyUniform` layout in
/// `sparse_classify.wgsl` (32 B std140, two `vec4<f32>`s).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct ClassifyUniform {
    /// `xyz` = chunk-local `bounds_min`, `w` = margin.
    bounds_min_margin: [f32; 4],
    /// `xyz` = chunk-local `bounds_max`, `w` = threshold_scale
    /// (reserved; 1.0 today, kept in the layout so the uniform stays
    /// 32 B and downstream margin-tuning experiments do not need an
    /// ABI bump).
    bounds_max_scale: [f32; 4],
}

/// Compiled classify + finalize pipelines. One instance is enough for
/// any number of [`SparseGrid`]s sharing the same sampler — the bind
/// group is rebuilt per [`record`] call so the pass is grid-agnostic.
pub struct ClassifyPass {
    classify_pipeline: wgpu::ComputePipeline,
    classify_bgl: wgpu::BindGroupLayout,
    sampler_bgl: wgpu::BindGroupLayout,
    finalize_pipeline: wgpu::ComputePipeline,
    finalize_bgl: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl ClassifyPass {
    /// Build the classify + finalize pipelines against `sampler_wgsl`
    /// (concatenated ahead of [`CLASSIFY_WGSL`]) and `sampler_bgl_entries`
    /// (used as the second bind group layout, `@group(1)`).
    pub fn new(
        device: &wgpu::Device,
        sampler_wgsl: &str,
        sampler_bgl_entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Self {
        let classify_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_sdf::sparse::classify::classify_bgl"),
            entries: &CLASSIFY_BGL_ENTRIES,
        });
        let sampler_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_sdf::sparse::classify::sampler_bgl"),
            entries: sampler_bgl_entries,
        });
        let classify_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_sdf::sparse::classify::classify_layout"),
            bind_group_layouts: &[Some(&classify_bgl), Some(&sampler_bgl)],
            immediate_size: 0,
        });
        let classify_src = format!("{sampler_wgsl}{CLASSIFY_WGSL}");
        let classify_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_sdf::sparse::classify::classify_shader"),
            source: wgpu::ShaderSource::Wgsl(classify_src.into()),
        });
        let classify_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_sdf::sparse::classify::classify_pipeline"),
            layout: Some(&classify_layout),
            module: &classify_module,
            entry_point: Some("classify_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let finalize_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_sdf::sparse::classify::finalize_bgl"),
            entries: &FINALIZE_BGL_ENTRIES,
        });
        let finalize_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_sdf::sparse::classify::finalize_layout"),
            bind_group_layouts: &[Some(&finalize_bgl)],
            immediate_size: 0,
        });
        let finalize_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_sdf::sparse::classify::finalize_shader"),
            source: wgpu::ShaderSource::Wgsl(CLASSIFY_FINALIZE_WGSL.into()),
        });
        let finalize_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_sdf::sparse::classify::finalize_pipeline"),
            layout: Some(&finalize_layout),
            module: &finalize_module,
            entry_point: Some("finalize_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_sdf::sparse::classify::uniform"),
            size: std::mem::size_of::<ClassifyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            classify_pipeline,
            classify_bgl,
            sampler_bgl,
            finalize_pipeline,
            finalize_bgl,
            uniform_buffer,
        }
    }

    /// Bind group layout the caller must use when assembling the
    /// sampler bind group passed to [`record`]. Same structural shape
    /// as `sampler_bgl_entries` from [`new`], but exposing this
    /// handle avoids relying on wgpu's structural-equality fallback
    /// for cross-handle layout matching.
    pub fn sampler_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.sampler_bgl
    }

    /// Record the classify + finalize compute passes into `encoder`.
    ///
    /// Encoder ordering inside this call:
    ///
    /// 1. `clear_buffer(needs_count, 0..4)` — zero the atomic counter.
    /// 2. `classify_main` dispatch — 64 workgroups × 64 threads.
    /// 3. `finalize_main` dispatch — 1 workgroup × 1 thread.
    ///
    /// `queue.write_buffer` for the uniform happens before the encoder
    /// commands run (wgpu serialises queue writes ahead of submitted
    /// command buffers within the same submission).
    ///
    /// The caller owns submitting the encoder and supplying the
    /// `sampler_bg` matching the sampler the pipeline was built with.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        sampler_bg: &wgpu::BindGroup,
        margin: f32,
    ) {
        let bounds = grid.bounds();
        let uniform = ClassifyUniform {
            bounds_min_margin: [bounds.min.x, bounds.min.y, bounds.min.z, margin],
            bounds_max_scale: [bounds.max.x, bounds.max.y, bounds.max.z, 1.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        // Zero the atomic counter via queue.write_buffer (staged ahead
        // of the encoder commands in the same submission). Avoids the
        // backend-specific feature gate some `encoder.clear_buffer`
        // implementations carry, and keeps the per-frame dispatch
        // overhead at one queue write + the two compute passes.
        queue.write_buffer(grid.needs_count_buffer(), 0, &[0u8; 4]);

        let classify_bg = self.create_classify_bg(device, grid);
        let finalize_bg = self.create_finalize_bg(device, grid);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ome_sdf::sparse::classify::classify_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.classify_pipeline);
            pass.set_bind_group(0, &classify_bg, &[]);
            pass.set_bind_group(1, sampler_bg, &[]);
            let workgroups = ROOT_CELLS.div_ceil(CLASSIFY_WORKGROUP_SIZE);
            pass.dispatch_workgroups(workgroups, 1, 1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ome_sdf::sparse::classify::finalize_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.finalize_pipeline);
            pass.set_bind_group(0, &finalize_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
    }

    fn create_classify_bg(
        &self,
        device: &wgpu::Device,
        grid: &SparseGrid,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_sdf::sparse::classify::classify_bg"),
            layout: &self.classify_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.root_indices_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: grid.needs_indices_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: grid.needs_count_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
            ],
        })
    }

    fn create_finalize_bg(
        &self,
        device: &wgpu::Device,
        grid: &SparseGrid,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_sdf::sparse::classify::finalize_bg"),
            layout: &self.finalize_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.needs_count_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid.needs_indirect_args_buffer().as_entire_binding(),
                },
            ],
        })
    }
}

/// Bind group layout entries for the classify pass `@group(0)`. Same
/// binding numbers used in `sparse_classify.wgsl`.
const CLASSIFY_BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 4] = [
    // root_indices read-only
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // needs_indices read_write
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // needs_count atomic (read_write storage)
    wgpu::BindGroupLayoutEntry {
        binding: 3,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // classify uniform
    wgpu::BindGroupLayoutEntry {
        binding: 4,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
];

/// Bind group layout entries for the finalize pass `@group(0)`.
const FINALIZE_BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 2] = [
    // needs_count read
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // indirect args read_write
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
];

#[cfg(test)]
mod tests;
