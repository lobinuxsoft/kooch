//! Classify pass — flag root cells whose centres fall within one
//! cell-diagonal of the sampled SDF surface (single-sample Lipschitz
//! cone test). One pipeline per LOD; each pipeline is gated on the
//! corresponding bit of `chunk_lod_mask`, so only LODs the chunk
//! activates do real work.
//!
//! GPU-driven: 1 SDF eval per root cell, 4096 evals per chunk, 64
//! workgroups × 64 threads. Output is an indirect-ready compaction
//! consumed by [`super::PopulatePass`] without a CPU readback in the
//! hot loop.
//!
//! # S7 — per-LOD pipelines
//!
//! [`ClassifyPass`] holds [`LOD_COUNT`] (= 4) compute pipelines, one
//! per LOD. Each pipeline pins the `CLASSIFY_LOD_IDX` WGSL override
//! to its level so the in-shader `chunk_lod_mask & (1 << lod)` test
//! folds to a constant. Bind groups are built per-record call against
//! the LOD's per-LOD `root_indices`, `needs_indices`, and `needs_count`
//! buffers.
//!
//! No finalize pass lives in this module any more — the indirect-args
//! derivation moved to [`super::PopulatePass`]'s populate-finalize so
//! the cascade's 16-pass chain stays tight (chunk_lod → classify[0..3]
//! → populate_finalize[0..3] → populate[0..3] → downsample[0..2]).
//!
//! [`LOD_COUNT`]: super::LOD_COUNT

use bytemuck::{Pod, Zeroable};

use super::{LOD_COUNT, ROOT_CELLS, ROOT_DIM, SparseGrid};

/// WGSL source of the classify pass body — `classify_main` plus the
/// `@group(0)` binding declarations. Concatenated with the sampler
/// fragment by [`ClassifyPass::new`].
pub const CLASSIFY_WGSL: &str = include_str!("../../shaders/sparse_classify.wgsl");

/// Workgroup size matching the `@workgroup_size(64)` annotation in
/// `sparse_classify.wgsl`. Kept as a Rust constant so the dispatch
/// math can't drift from the shader.
pub const CLASSIFY_WORKGROUP_SIZE: u32 = 64;

/// Default cone-test margin. `1.0` is the exact Lipschitz cone radius
/// for unit-Lipschitz SDFs. Bump via [`ClassifyPass::record`]'s
/// `margin` argument when calibrating non-Lipschitz samplers.
pub const DEFAULT_MARGIN: f32 = 1.0;

/// Uniform mirror — must match the WGSL `ClassifyUniform` layout in
/// `sparse_classify.wgsl` (32 B std140, two `vec4<f32>`s).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct ClassifyUniform {
    /// `xyz` = chunk-local `bounds_min`, `w` = margin.
    bounds_min_margin: [f32; 4],
    /// `xyz` = chunk-local `bounds_max`, `w` = threshold_scale (1.0).
    bounds_max_scale: [f32; 4],
}

/// Compiled classify pipelines (one per LOD) plus the shared bind
/// group layouts and the uniform buffer. One instance is enough for
/// any number of [`SparseGrid`]s sharing the same sampler.
pub struct ClassifyPass {
    classify_pipelines: [wgpu::ComputePipeline; LOD_COUNT as usize],
    classify_bgl: wgpu::BindGroupLayout,
    sampler_bgl: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl ClassifyPass {
    /// Build the per-LOD classify pipelines against `sampler_wgsl`
    /// (concatenated ahead of [`CLASSIFY_WGSL`]) and `sampler_bgl_entries`
    /// (used as the second bind group layout, `@group(1)`).
    ///
    /// Each LOD's pipeline pins `CLASSIFY_LOD_IDX` to the LOD index so
    /// the in-shader `1u << CLASSIFY_LOD_IDX` test folds to a constant.
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

        let classify_pipelines = std::array::from_fn(|lod_idx| {
            let label = format!("ome_sdf::sparse::classify::classify_pipeline_lod{lod_idx}");
            let constants: &[(&str, f64)] = &[
                ("CLASSIFY_LOD_IDX", lod_idx as f64),
                ("CLASSIFY_ROOT_DIM", ROOT_DIM as f64),
                ("CLASSIFY_ROOT_CELLS", ROOT_CELLS as f64),
            ];
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(&label),
                layout: Some(&classify_layout),
                module: &classify_module,
                entry_point: Some("classify_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    zero_initialize_workgroup_memory: true,
                },
                cache: None,
            })
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_sdf::sparse::classify::uniform"),
            size: std::mem::size_of::<ClassifyUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            classify_pipelines,
            classify_bgl,
            sampler_bgl,
            uniform_buffer,
        }
    }

    /// Bind group layout the caller must use when assembling the
    /// sampler bind group passed to [`record`]. Same structural shape
    /// as `sampler_bgl_entries` from [`new`].
    pub fn sampler_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.sampler_bgl
    }

    /// Record a classify dispatch for one LOD into `encoder`.
    ///
    /// Encoder ordering inside this call:
    ///
    /// 1. `clear` `needs_count[lod_idx]` to 0 (via queue write).
    /// 2. `classify_main` dispatch — 64 workgroups × 64 threads.
    ///
    /// `queue.write_buffer` for the uniform happens before the encoder
    /// commands run (wgpu serialises queue writes ahead of submitted
    /// command buffers within the same submission).
    ///
    /// Caller invariant: the chunk_lod_mask buffer must have been
    /// written by [`super::ChunkLodPass::record`] earlier in the same
    /// encoder (or in a previous submission). If the LOD's bit is
    /// unset, the dispatch becomes a no-op and `needs_count[lod_idx]`
    /// stays at 0 — the populate pass at this LOD then dispatches over
    /// zero workgroups, harmlessly.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        sampler_bg: &wgpu::BindGroup,
        lod_idx: u32,
        margin: f32,
    ) {
        let bounds = grid.bounds();
        let uniform = ClassifyUniform {
            bounds_min_margin: [bounds.min.x, bounds.min.y, bounds.min.z, margin],
            bounds_max_scale: [bounds.max.x, bounds.max.y, bounds.max.z, 1.0],
        };
        // Same uniform shared across all LODs — the cone test is LOD-
        // independent (root grid resolution does not change with LOD).
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
        // Zero the LOD's atomic counter via queue.write_buffer.
        queue.write_buffer(grid.needs_count_buffer(lod_idx), 0, &[0u8; 4]);

        let classify_bg = self.create_classify_bg(device, grid, lod_idx);

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_sdf::sparse::classify::classify_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.classify_pipelines[lod_idx as usize]);
        pass.set_bind_group(0, &classify_bg, &[]);
        pass.set_bind_group(1, sampler_bg, &[]);
        let workgroups = ROOT_CELLS.div_ceil(CLASSIFY_WORKGROUP_SIZE);
        pass.dispatch_workgroups(workgroups, 1, 1);
    }

    fn create_classify_bg(
        &self,
        device: &wgpu::Device,
        grid: &SparseGrid,
        lod_idx: u32,
    ) -> wgpu::BindGroup {
        let label = format!("ome_sdf::sparse::classify::classify_bg_lod{lod_idx}");
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&label),
            layout: &self.classify_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.root_indices_buffer(lod_idx).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: grid.needs_indices_buffer(lod_idx).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: grid.needs_count_buffer(lod_idx).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: grid.chunk_lod_mask_buffer().as_entire_binding(),
                },
            ],
        })
    }
}

/// Bind group layout entries for the classify pass `@group(0)`. Same
/// binding numbers used in `sparse_classify.wgsl`. S7 added binding 5
/// (`chunk_lod_mask`) so the in-shader gating reads the per-chunk LOD
/// activity bitmask.
const CLASSIFY_BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 5] = [
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
    // chunk_lod_mask (read storage)
    wgpu::BindGroupLayoutEntry {
        binding: 5,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
];

#[cfg(test)]
mod tests;
