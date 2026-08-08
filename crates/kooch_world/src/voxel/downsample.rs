//! Downsample cascade — fills LODs 1..=3 by box-filtering the
//! preceding LOD. One pipeline per cascade pair, dispatched indirect
//! over LOD 0's `populate_indirect_args` (so each cascade sees one
//! workgroup per cell that classify marked at LOD 0 — the only LOD
//! the orchestrator runs classify + populate on, since the marked
//! cell set is LOD-independent).
//!
//! See `shaders/sparse_downsample.wgsl` for the box-filter math and
//! the skirt-clamp invariant. This module is the host-side
//! orchestration: pipeline construction with per-cascade overrides
//! plus bind-group assembly.
//!
//! # Caller invariant
//!
//! [`DownsamplePass::record_cascade`] requires that
//! [`super::PopulatePass::record`] (or its `record_finalize` +
//! `record_populate` halves) has run for the source LOD earlier in
//! the same encoder. The cascade reads
//! `grid.populate_indirect_args_buffer(lod_src)` (already populated
//! by populate-finalize), `grid.needs_indices_buffer(lod_src)`,
//! `grid.needs_count_buffer(lod_src)`, and
//! `grid.subgrid_pool_view(lod_src)` (already filled by the LOD's
//! populate stage).

use super::{LOD_LEVELS, SparseGrid};

/// WGSL source of the downsample cascade pass.
pub const DOWNSAMPLE_WGSL: &str = include_str!("../../shaders/sparse_downsample.wgsl");

/// Number of cascade pairs (`LOD_COUNT - 1`).
pub const CASCADE_COUNT: usize = (super::LOD_COUNT as usize) - 1;

/// Workgroup size matching the `@workgroup_size(64)` annotation in
/// `sparse_downsample.wgsl`.
pub const DOWNSAMPLE_WORKGROUP_SIZE: u32 = 64;

/// Compiled per-cascade downsample pipelines plus the shared bind
/// group layout. One instance is enough per device — bind groups are
/// rebuilt per [`record_cascade`] call.
pub struct DownsamplePass {
    pipelines: [wgpu::ComputePipeline; CASCADE_COUNT],
    bgl: wgpu::BindGroupLayout,
}

impl DownsamplePass {
    /// Build the three downsample pipelines (cascades 0→1, 1→2, 2→3).
    /// Each pipeline pins source-tile and destination-tile geometry
    /// in its override constants.
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kooch_world::voxel::downsample::bgl"),
            entries: &BGL_ENTRIES,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kooch_world::voxel::downsample::layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kooch_world::voxel::downsample::shader"),
            source: wgpu::ShaderSource::Wgsl(DOWNSAMPLE_WGSL.into()),
        });

        let pipelines = std::array::from_fn(|cascade_idx| {
            let lod_src = cascade_idx;
            let lod_dst = cascade_idx + 1;
            let src = LOD_LEVELS[lod_src];
            let dst = LOD_LEVELS[lod_dst];
            let label = format!(
                "kooch_world::voxel::downsample::pipeline_c{cascade_idx}_lod{lod_src}_to_lod{lod_dst}",
            );
            let constants: &[(&str, f64)] = &[
                ("DOWNSAMPLE_DST_SUBGRID_DIM", dst.subgrid_dim as f64),
                ("DOWNSAMPLE_DST_TILE_DIM", dst.tile_dim as f64),
                ("DOWNSAMPLE_DST_TILE_VOXELS", dst.tile_voxels() as f64),
                ("DOWNSAMPLE_DST_ATLAS_TILES_X", dst.atlas_tiles_x as f64),
                ("DOWNSAMPLE_DST_ATLAS_TILES_Y", dst.atlas_tiles_y as f64),
                ("DOWNSAMPLE_SRC_TILE_DIM", src.tile_dim as f64),
                ("DOWNSAMPLE_SRC_ATLAS_TILES_X", src.atlas_tiles_x as f64),
                ("DOWNSAMPLE_SRC_ATLAS_TILES_Y", src.atlas_tiles_y as f64),
            ];
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(&label),
                layout: Some(&layout),
                module: &module,
                entry_point: Some("downsample_main"),
                compilation_options: wgpu::PipelineCompilationOptions {
                    constants,
                    zero_initialize_workgroup_memory: true,
                },
                cache: None,
            })
        });

        Self { pipelines, bgl }
    }

    /// Record the cascade `cascade_idx` (`0` = LOD 0→1, `1` = LOD
    /// 1→2, `2` = LOD 2→3) into `encoder`. All cascades share the
    /// same dispatch shape (`populate_indirect_args[0]` =
    /// `[needs_count_lod0, 1, 1]`) and the same `needs_indices` /
    /// `needs_count` source — LOD 0's, since the marked cell set is
    /// LOD-independent and only LOD 0 actually runs classify in the
    /// canonical cascade.
    pub fn record_cascade(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        cascade_idx: u32,
    ) {
        let lod_src = cascade_idx;
        let lod_dst = cascade_idx + 1;
        let bg = self.create_bg(device, grid, lod_src, lod_dst);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kooch_world::voxel::downsample::pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipelines[cascade_idx as usize]);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups_indirect(grid.populate_indirect_args_buffer(0), 0);
    }

    fn create_bg(
        &self,
        device: &wgpu::Device,
        grid: &SparseGrid,
        lod_src: u32,
        lod_dst: u32,
    ) -> wgpu::BindGroup {
        let label = format!("kooch_world::voxel::downsample::bg_lod{lod_src}_to_lod{lod_dst}");
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(&label),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: grid.root_indices_buffer(lod_src).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid.root_indices_buffer(lod_dst).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(grid.subgrid_pool_view(lod_src)),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(grid.subgrid_pool_view(lod_dst)),
                },
                // needs_indices / needs_count come from LOD 0
                // regardless of cascade — the canonical cascade only
                // runs classify at LOD 0, and the marked cell set is
                // LOD-independent.
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: grid.needs_indices_buffer(0).as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: grid.needs_count_buffer(0).as_entire_binding(),
                },
            ],
        })
    }
}

const BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 6] = [
    // src_root_indices (read storage)
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
    // dst_root_indices (read_write storage)
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
    // src_pool (sampled texture, but we use textureLoad — `Float` non-filterable is OK)
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D3,
            multisampled: false,
        },
        count: None,
    },
    // dst_pool (storage texture, write-only, R16Float)
    wgpu::BindGroupLayoutEntry {
        binding: 3,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format: wgpu::TextureFormat::R16Float,
            view_dimension: wgpu::TextureViewDimension::D3,
        },
        count: None,
    },
    // needs_indices_src (read storage)
    wgpu::BindGroupLayoutEntry {
        binding: 4,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // needs_count_src (read storage)
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
