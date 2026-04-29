//! Populate pass — allocate one subgrid from the free-list pool per
//! marked root cell (output of the classify pass) and fill the 16³
//! voxels with sampler values, all in a single GPU dispatch.
//!
//! Compose with [`super::ClassifyPass::record`] within the same
//! command encoder (or an earlier submission of the same chunk):
//! populate consumes `needs_indices[0..needs_count]` written by
//! classify and is undefined behaviour without that producer.
//!
//! # Approach D — single-pass populate-and-allocate with SLM coordination
//!
//! 1 workgroup per marked cell, 256 threads collaborating on the 4096
//! voxels of that cell's subgrid (inner serial stride loop, 16 voxels
//! per thread). Thread 0 pops one subgrid index off the atomic free
//! list, broadcasts via `var<workgroup>` after a `workgroupBarrier`,
//! and the rest of the workgroup samples the SDF in parallel. Thread
//! 0 writes `root_indices[cell_idx] = subgrid_idx` last, after a
//! second barrier, so the pool writes happen-before the root pointer
//! publishes them.
//!
//! Why one allocation per workgroup (rather than per-thread / per-cell
//! batched in a separate pass): keeps the dispatch count flat
//! (single indirect dispatch over `needs_count` workgroups), keeps the
//! pop hot loop confined to 1/256 of the threads (one CAS contender
//! per workgroup, not per voxel), and keeps the freelist ABI shared
//! with future allocate / free passes (#S5–S7) without adding a
//! split-buffer "pending allocation" intermediate.
//!
//! # Workgroup-size choice (256)
//!
//! 4096 voxels / 256 threads = 16 voxels per thread, serial inner
//! loop. Hits high occupancy on both targets:
//! - Steam Deck (RDNA 2, wavefront 64) → 4 waves/wg
//! - RX 9070 XT (RDNA 4, wavefront 32) → 8 waves/wg
//!
//! # Indirect dispatch — finalize shader reuse
//!
//! Reuses [`super::CLASSIFY_FINALIZE_WGSL`] with the
//! `FINALIZE_WORKGROUP_SIZE` pipeline override pinned to `1u`. The
//! classify path overrides it to `64u` (downstream allocate consumer
//! is `@workgroup_size(64)`); populate dispatches 1 workgroup per
//! marked cell, so the divisor is `1`. Sharing the finalize WGSL
//! avoids a second copy that would drift the moment one consumer's
//! workgroup size changes.
//!
//! # Caller invariant
//!
//! [`PopulatePass::record`] requires that
//! [`super::ClassifyPass::record`] has run earlier in the same command
//! encoder (or in a previously-submitted command buffer for this
//! chunk) — populate reads `needs_indices` and `needs_count` written
//! there. wgpu inserts the implicit storage-buffer memory barrier
//! between consecutive compute passes, and queue submission order
//! provides cross-frame ordering.

mod bindings;

use bindings::{FINALIZE_BGL_ENTRIES, POPULATE_BGL_ENTRIES};
use bytemuck::{Pod, Zeroable};

use super::{CLASSIFY_FINALIZE_WGSL, SPARSE_FREELIST_WGSL, SparseGrid};

/// WGSL source of the populate pass body — `populate_main` plus the
/// `@group(0)` binding declarations starting at `binding(5)`. Built
/// into the compiled shader by [`PopulatePass::new`] as
/// `format!("{SPARSE_FREELIST_WGSL}{sampler_wgsl}{POPULATE_WGSL}")`.
pub const POPULATE_WGSL: &str = include_str!("../../shaders/sparse_populate.wgsl");

/// Workgroup size matching the `@workgroup_size(256)` annotation in
/// `sparse_populate.wgsl`. Kept as a Rust constant so the dispatch
/// math + tests can reference the same value the shader compiles to.
pub const POPULATE_WORKGROUP_SIZE: u32 = 256;

/// `FINALIZE_WORKGROUP_SIZE` override the populate finalize pipeline
/// pins. Populate dispatches `n` workgroups per `n` marked cells, so
/// the indirect-args x is `⌈n / 1⌉ = n`.
const POPULATE_FINALIZE_DIVISOR: f64 = 1.0;

/// Uniform mirror — must match the WGSL `PopulateUniform` layout in
/// `sparse_populate.wgsl` (32 B std140, two `vec4<f32>`s).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct PopulateUniform {
    /// `xyz` = chunk-local `bounds_min`, `w` reserved.
    bounds_min: [f32; 4],
    /// `xyz` = chunk-local `bounds_max`, `w` reserved.
    bounds_max: [f32; 4],
}

/// Compiled populate + finalize pipelines. One instance is enough for
/// any number of [`SparseGrid`]s sharing the same sampler — bind
/// groups are rebuilt per [`record`] call so the pass is grid-agnostic.
pub struct PopulatePass {
    populate_pipeline: wgpu::ComputePipeline,
    populate_bgl: wgpu::BindGroupLayout,
    sampler_bgl: wgpu::BindGroupLayout,
    finalize_pipeline: wgpu::ComputePipeline,
    finalize_bgl: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl PopulatePass {
    /// Build the populate + finalize pipelines against `sampler_wgsl`
    /// (concatenated between the freelist helpers and the populate
    /// body) and `sampler_bgl_entries` (used as the second bind group
    /// layout, `@group(1)`).
    pub fn new(
        device: &wgpu::Device,
        sampler_wgsl: &str,
        sampler_bgl_entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Self {
        let populate_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_sdf::sparse::populate::populate_bgl"),
            entries: &POPULATE_BGL_ENTRIES,
        });
        let sampler_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_sdf::sparse::populate::sampler_bgl"),
            entries: sampler_bgl_entries,
        });
        let populate_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_sdf::sparse::populate::populate_layout"),
            bind_group_layouts: &[Some(&populate_bgl), Some(&sampler_bgl)],
            immediate_size: 0,
        });
        let populate_src =
            format!("{SPARSE_FREELIST_WGSL}{sampler_wgsl}{POPULATE_WGSL}");
        let populate_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_sdf::sparse::populate::populate_shader"),
            source: wgpu::ShaderSource::Wgsl(populate_src.into()),
        });
        let populate_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_sdf::sparse::populate::populate_pipeline"),
            layout: Some(&populate_layout),
            module: &populate_module,
            entry_point: Some("populate_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let finalize_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_sdf::sparse::populate::finalize_bgl"),
            entries: &FINALIZE_BGL_ENTRIES,
        });
        let finalize_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_sdf::sparse::populate::finalize_layout"),
            bind_group_layouts: &[Some(&finalize_bgl)],
            immediate_size: 0,
        });
        let finalize_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_sdf::sparse::populate::finalize_shader"),
            source: wgpu::ShaderSource::Wgsl(CLASSIFY_FINALIZE_WGSL.into()),
        });
        let finalize_constants: &[(&str, f64)] =
            &[("FINALIZE_WORKGROUP_SIZE", POPULATE_FINALIZE_DIVISOR)];
        let finalize_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_sdf::sparse::populate::finalize_pipeline"),
            layout: Some(&finalize_layout),
            module: &finalize_module,
            entry_point: Some("finalize_main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: finalize_constants,
                zero_initialize_workgroup_memory: true,
            },
            cache: None,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_sdf::sparse::populate::uniform"),
            size: std::mem::size_of::<PopulateUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            populate_pipeline,
            populate_bgl,
            sampler_bgl,
            finalize_pipeline,
            finalize_bgl,
            uniform_buffer,
        }
    }

    /// Bind group layout the caller must use when assembling the
    /// sampler bind group passed to [`record`]. Same structural shape
    /// as `sampler_bgl_entries` from [`new`], but exposing this handle
    /// avoids relying on wgpu's structural-equality fallback for
    /// cross-handle layout matching.
    pub fn sampler_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.sampler_bgl
    }

    /// Record the populate-finalize + populate compute passes into
    /// `encoder`.
    ///
    /// **Caller invariant:** [`super::ClassifyPass::record`] must have
    /// run earlier in this same `encoder` (or in a previously-submitted
    /// command buffer for `grid`). Populate reads
    /// `grid.needs_indices_buffer()` and `grid.needs_count_buffer()`
    /// written by classify; without that producer the pass dispatches
    /// over stale or zeroed data.
    ///
    /// Encoder ordering inside this call:
    ///
    /// 1. `finalize_main` dispatch — derive
    ///    `[needs_count, 1, 1]` into `populate_indirect_args_buffer`.
    /// 2. `populate_main` indirect dispatch — one workgroup per marked
    ///    cell, 256 threads cooperating on the 4096 voxels.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        sampler_bg: &wgpu::BindGroup,
    ) {
        let bounds = grid.bounds();
        let uniform = PopulateUniform {
            bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z, 0.0],
            bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let populate_bg = self.create_populate_bg(device, grid);
        let finalize_bg = self.create_finalize_bg(device, grid);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ome_sdf::sparse::populate::finalize_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.finalize_pipeline);
            pass.set_bind_group(0, &finalize_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ome_sdf::sparse::populate::populate_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.populate_pipeline);
            pass.set_bind_group(0, &populate_bg, &[]);
            pass.set_bind_group(1, sampler_bg, &[]);
            pass.dispatch_workgroups_indirect(grid.populate_indirect_args_buffer(), 0);
        }
    }

    fn create_populate_bg(
        &self,
        device: &wgpu::Device,
        grid: &SparseGrid,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_sdf::sparse::populate::populate_bg"),
            layout: &self.populate_bgl,
            entries: &[
                // Freelist bindings — must match SPARSE_FREELIST_WGSL.
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.free_list_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid.counters_buffer().as_entire_binding(),
                },
                // Populate-specific bindings (5..=9).
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: grid.root_indices_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: grid.subgrid_pool_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: grid.needs_indices_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: grid.needs_count_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 9,
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
            label: Some("ome_sdf::sparse::populate::finalize_bg"),
            layout: &self.finalize_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.needs_count_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid.populate_indirect_args_buffer().as_entire_binding(),
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests;
