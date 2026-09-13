//! Populate: per marked cell thread 0 pops a subgrid, 256 threads sample the SDF, and the root
//! pointer publishes last. [`PopulatePass::record`] needs [`super::ClassifyPass::record`] first.

mod bindings;

use bindings::{FINALIZE_BGL_ENTRIES, POPULATE_BGL_ENTRIES};
use bytemuck::{Pod, Zeroable};

use super::{LOD_COUNT, LOD_LEVELS, ROOT_DIM, SPARSE_FREELIST_WGSL, SparseGrid};

/// WGSL source of the populate pass body.
pub const POPULATE_WGSL: &str = include_str!("../../shaders/sparse_populate.wgsl");

/// WGSL source of the finalize pass — shared with the (now-retired)
/// classify-finalize. Owned here because populate is the only
/// consumer left in the cascade.
pub(crate) const FINALIZE_WGSL: &str = include_str!("../../shaders/sparse_classify_finalize.wgsl");

/// Workgroup size matching the `@workgroup_size(256)` annotation in
/// `sparse_populate.wgsl`. Constant across LODs (over-provisioned at
/// LOD 3, where `tile_voxels = 27` leaves most threads idle).
pub const POPULATE_WORKGROUP_SIZE: u32 = 256;

/// `FINALIZE_WORKGROUP_SIZE` override pinned for the populate
/// finalize pipeline. Populate dispatches `n` workgroups per `n`
/// marked cells, so the indirect-args x is `⌈n / 1⌉ = n`.
const POPULATE_FINALIZE_DIVISOR: f64 = 1.0;

/// Uniform mirror — must match the WGSL `PopulateUniform` layout in
/// `sparse_populate.wgsl` (32 B std140, two `vec4<f32>`s).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct PopulateUniform {
    bounds_min: [f32; 4],
    bounds_max: [f32; 4],
}

/// Pipelines only; bind groups are rebuilt per record, so one instance serves every [`SparseGrid`]
/// sharing a sampler.
pub struct PopulatePass {
    populate_pipelines: [wgpu::ComputePipeline; LOD_COUNT as usize],
    populate_bgl: wgpu::BindGroupLayout,
    sampler_bgl: wgpu::BindGroupLayout,
    finalize_pipeline: wgpu::ComputePipeline,
    finalize_bgl: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl PopulatePass {
    /// `sampler_wgsl` goes between the freelist helpers and the populate body;
    /// `sampler_bgl_entries` is `@group(1)`.
    pub fn new(
        device: &wgpu::Device,
        sampler_wgsl: &str,
        sampler_bgl_entries: &[wgpu::BindGroupLayoutEntry],
    ) -> Self {
        let populate_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kooch_world::voxel::populate::populate_bgl"),
            entries: &POPULATE_BGL_ENTRIES,
        });
        let sampler_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kooch_world::voxel::populate::sampler_bgl"),
            entries: sampler_bgl_entries,
        });
        let populate_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kooch_world::voxel::populate::populate_layout"),
            bind_group_layouts: &[Some(&populate_bgl), Some(&sampler_bgl)],
            immediate_size: 0,
        });
        let populate_src = format!("{SPARSE_FREELIST_WGSL}{sampler_wgsl}{POPULATE_WGSL}");
        let populate_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kooch_world::voxel::populate::populate_shader"),
            source: wgpu::ShaderSource::Wgsl(populate_src.into()),
        });

        let populate_pipelines = std::array::from_fn(|lod_idx| {
            let lod = LOD_LEVELS[lod_idx];
            let label = format!("kooch_world::voxel::populate::populate_pipeline_lod{lod_idx}");
            let constants: &[(&str, f64)] = &[
                ("POPULATE_SUBGRID_DIM", lod.subgrid_dim as f64),
                ("POPULATE_TILE_DIM", lod.tile_dim as f64),
                ("POPULATE_TILE_VOXELS", lod.tile_voxels() as f64),
                ("POPULATE_ATLAS_TILES_X", lod.atlas_tiles_x as f64),
                ("POPULATE_ATLAS_TILES_Y", lod.atlas_tiles_y as f64),
                ("POPULATE_ROOT_DIM", ROOT_DIM as f64),
            ];
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(&label),
                layout: Some(&populate_layout),
                module: &populate_module,
                entry_point: Some("populate_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    zero_initialize_workgroup_memory: true,
                },
                cache: None,
            })
        });

        let finalize_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kooch_world::voxel::populate::finalize_bgl"),
            entries: &FINALIZE_BGL_ENTRIES,
        });
        let finalize_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kooch_world::voxel::populate::finalize_layout"),
            bind_group_layouts: &[Some(&finalize_bgl)],
            immediate_size: 0,
        });
        let finalize_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kooch_world::voxel::populate::finalize_shader"),
            source: wgpu::ShaderSource::Wgsl(FINALIZE_WGSL.into()),
        });
        let finalize_constants: &[(&str, f64)] =
            &[("FINALIZE_WORKGROUP_SIZE", POPULATE_FINALIZE_DIVISOR)];
        let finalize_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kooch_world::voxel::populate::finalize_pipeline"),
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
            label: Some("kooch_world::voxel::populate::uniform"),
            size: std::mem::size_of::<PopulateUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            populate_pipelines,
            populate_bgl,
            sampler_bgl,
            finalize_pipeline,
            finalize_bgl,
            uniform_buffer,
        }
    }

    /// Bind group layout the caller must use when assembling the
    /// sampler bind group passed to [`Self::record`].
    pub fn sampler_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.sampler_bgl
    }

    /// Records populate-finalize, then populate indirect, for `lod_idx`.
    /// [`super::ClassifyPass::record`] must have run at the same LOD, or it dispatches over stale
    /// data.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        sampler_bg: &wgpu::BindGroup,
        lod_idx: u32,
    ) {
        let bounds = grid.bounds();
        let uniform = PopulateUniform {
            bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z, 0.0],
            bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let populate_bg = self.create_populate_bg(device, grid, lod_idx);
        let finalize_bg = self.create_finalize_bg(device, grid, lod_idx);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kooch_world::voxel::populate::finalize_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.finalize_pipeline);
            pass.set_bind_group(0, &finalize_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kooch_world::voxel::populate::populate_pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.populate_pipelines[lod_idx as usize]);
            pass.set_bind_group(0, &populate_bg, &[]);
            pass.set_bind_group(1, sampler_bg, &[]);
            pass.dispatch_workgroups_indirect(grid.populate_indirect_args_buffer(lod_idx), 0);
        }
    }

    /// Record only the finalize stage for `lod_idx`. Useful for the
    /// orchestrator that batches all LODs' finalize passes ahead of
    /// the populate dispatches.
    pub fn record_finalize(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        lod_idx: u32,
    ) {
        let finalize_bg = self.create_finalize_bg(device, grid, lod_idx);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kooch_world::voxel::populate::finalize_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.finalize_pipeline);
        pass.set_bind_group(0, &finalize_bg, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }

    /// Record only the populate stage for `lod_idx`. Used by the
    /// orchestrator after all LODs' finalize stages have run.
    pub fn record_populate(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        sampler_bg: &wgpu::BindGroup,
        lod_idx: u32,
    ) {
        let bounds = grid.bounds();
        let uniform = PopulateUniform {
            bounds_min: [bounds.min.x, bounds.min.y, bounds.min.z, 0.0],
            bounds_max: [bounds.max.x, bounds.max.y, bounds.max.z, 0.0],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let populate_bg = self.create_populate_bg(device, grid, lod_idx);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kooch_world::voxel::populate::populate_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.populate_pipelines[lod_idx as usize]);
        pass.set_bind_group(0, &populate_bg, &[]);
        pass.set_bind_group(1, sampler_bg, &[]);
        pass.dispatch_workgroups_indirect(grid.populate_indirect_args_buffer(lod_idx), 0);
    }

    fn create_populate_bg(
        &self,
        device: &wgpu::Device,
        grid: &SparseGrid,
        lod_idx: u32,
    ) -> wgpu::BindGroup {
        let label = format!("kooch_world::voxel::populate::populate_bg_lod{lod_idx}");
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&label),
            layout: &self.populate_bgl,
            entries: &[
                // Freelist bindings — must match SPARSE_FREELIST_WGSL.
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.free_list_buffer(lod_idx).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid.counters_buffer(lod_idx).as_entire_binding(),
                },
                // Populate-specific bindings (5..=9).
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: grid.root_indices_buffer(lod_idx).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(grid.subgrid_pool_view(lod_idx)),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: grid.needs_indices_buffer(lod_idx).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 8,
                    resource: grid.needs_count_buffer(lod_idx).as_entire_binding(),
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
        lod_idx: u32,
    ) -> wgpu::BindGroup {
        let label = format!("kooch_world::voxel::populate::finalize_bg_lod{lod_idx}");
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&label),
            layout: &self.finalize_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.needs_count_buffer(lod_idx).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid
                        .populate_indirect_args_buffer(lod_idx)
                        .as_entire_binding(),
                },
            ],
        })
    }
}

#[cfg(test)]
mod tests;
