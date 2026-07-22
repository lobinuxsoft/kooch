//! Chunk-LOD selection pass — picks which LODs are active for each
//! chunk based on its distance to a global "active origin" (player
//! position) and writes the resulting bitmask into
//! `grid.chunk_lod_mask_buffer()`.
//!
//! # Output
//!
//! `chunk_lod_mask: u32` (single chunk today, `array<u32>` once
//! multi-chunk lands per #313). Bit `i` set ⇒ LOD `i` is active.
//! Bit 0 is always set — the downsample cascade reads LOD 0 as the
//! source for every higher LOD, so LOD 0 must always be populated.
//!
//! # Encoder ordering
//!
//! `ChunkLodPass::record` runs first in the cascade — every
//! downstream pipeline (classify, populate, downsample) reads
//! `chunk_lod_mask` and depends on it being up-to-date for this
//! frame's active origin.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use ome_core::Aabb;

use super::SparseGrid;

/// WGSL source of the chunk-LOD compute pass.
pub const CHUNK_LOD_WGSL: &str = include_str!("../../shaders/sparse_chunk_lod.wgsl");

/// Default LOD distance thresholds (metres).
///
/// - `[0]` = 100 m  → boundary between LOD 0 and LOD 1
/// - `[1]` = 500 m  → boundary between LOD 1 and LOD 2
/// - `[2]` = 2000 m → boundary between LOD 2 and LOD 3
///
/// Tuned for the planet-scale viewing distance regime: LOD 0
/// (finest) for the player's immediate ~100 m bubble, LOD 3
/// (coarsest) for everything beyond 2 km.
pub const DEFAULT_LOD_DISTANCE_THRESHOLDS: [f32; 3] = [100.0, 500.0, 2000.0];

/// Uniform mirror of WGSL `ChunkLodUniform`. 48 B std140 (three
/// `vec4<f32>`s).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
struct ChunkLodUniform {
    active_origin: [f32; 4],
    lod_distance_thresholds: [f32; 4],
    chunk_center_radius: [f32; 4],
}

/// Compiled chunk-LOD pipeline. One instance is enough per frame —
/// the bind group is rebuilt per [`record`] call so the pass is
/// grid-agnostic.
pub struct ChunkLodPass {
    pipeline: wgpu::ComputePipeline,
    bgl: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
}

impl ChunkLodPass {
    /// Build the pipeline + uniform buffer.
    pub fn new(device: &wgpu::Device) -> Self {
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ome_world::voxel::chunk_lod::bgl"),
            entries: &BGL_ENTRIES,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ome_world::voxel::chunk_lod::layout"),
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ome_world::voxel::chunk_lod::shader"),
            source: wgpu::ShaderSource::Wgsl(CHUNK_LOD_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ome_world::voxel::chunk_lod::pipeline"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("chunk_lod_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_world::voxel::chunk_lod::uniform"),
            size: std::mem::size_of::<ChunkLodUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            bgl,
            uniform_buffer,
        }
    }

    /// Stage the per-frame uniform and dispatch the chunk-LOD compute
    /// pass. `active_origin` is the world-space position relative to
    /// which LOD distances are measured (player / camera). `bounds`
    /// is `grid.bounds()` — the chunk centre is derived from it.
    /// `thresholds` is `(lod0_to_1, lod1_to_2, lod2_to_3)` distances
    /// in world units.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        grid: &SparseGrid,
        active_origin: Vec3,
        thresholds: [f32; 3],
    ) {
        let bounds = grid.bounds();
        let centre = chunk_centre(bounds);
        let radius = chunk_radius(bounds);
        let uniform = ChunkLodUniform {
            active_origin: [active_origin.x, active_origin.y, active_origin.z, 0.0],
            lod_distance_thresholds: [thresholds[0], thresholds[1], thresholds[2], 0.0],
            chunk_center_radius: [centre.x, centre.y, centre.z, radius],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ome_world::voxel::chunk_lod::bg"),
            layout: &self.bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: grid.chunk_lod_mask_buffer().as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ome_world::voxel::chunk_lod::pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(1, 1, 1);
    }
}

fn chunk_centre(bounds: Aabb) -> Vec3 {
    (bounds.min + bounds.max) * 0.5
}

fn chunk_radius(bounds: Aabb) -> f32 {
    let half_extent = (bounds.max - bounds.min) * 0.5;
    half_extent.length()
}

const BGL_ENTRIES: [wgpu::BindGroupLayoutEntry; 2] = [
    // chunk_lod_uniform
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    },
    // chunk_lod_mask (read_write storage)
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
