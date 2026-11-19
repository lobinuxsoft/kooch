//! Meshing (#313): surface nets over each generated block's tile, in two compute passes. The count
//! pass says what each block's mesh takes of the meshlet pool, a frame later the pool reserves it,
//! and the write pass fills those ranges in place; only the counts reach the CPU.

use bytemuck::{Pod, Zeroable};
use kooch_render::meshlet::{GpuMeshSizes, MeshDescriptor, PoolBuffers};

use super::TILE_SAMPLES;
use super::atlas::BlockAtlas;

#[cfg(test)]
mod tests;

const NETS_WGSL: &str = include_str!("../shaders/nets.wgsl");
const COUNT_WGSL: &str = include_str!("../shaders/count.wgsl");
const WRITE_WGSL: &str = include_str!("../shaders/write.wgsl");

/// Words the count pass gives a block: vertices, meshlets, meshlet vertices, triangle bytes.
pub(super) const COUNT_WORDS: u64 = 4;

const _: () = assert!(TILE_SAMPLES == 18, "nets.wgsl hard-codes an 18³ tile");

/// What a block's mesh takes of the pool, as the count pass found it.
pub(super) fn sizes_of(counts: [u32; 4]) -> GpuMeshSizes {
    GpuMeshSizes {
        vertices: counts[0],
        meshlets: counts[1],
        meshlet_vertices: counts[2],
        triangle_bytes: counts[3],
    }
}

/// One block for the write pass: where its ranges start, and what the count pass found.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct WriteJob {
    slot: u32,
    voxel: f32,
    first_vertex: u32,
    first_meshlet_vertex: u32,
    first_byte: u32,
    first_meshlet: u32,
    vertices: u32,
    meshlets: u32,
    meshlet_vertices: u32,
    bytes: u32,
    _pad: [u32; 2],
}

impl WriteJob {
    pub fn new(slot: u32, voxel: f32, sizes: GpuMeshSizes, at: &MeshDescriptor) -> Self {
        Self {
            slot,
            voxel,
            first_vertex: at.vertex_offset,
            first_meshlet_vertex: at.meshlet_vertex_offset,
            first_byte: at.meshlet_triangle_offset,
            first_meshlet: at.first_meshlet,
            vertices: sizes.vertices,
            meshlets: sizes.meshlets,
            meshlet_vertices: sizes.meshlet_vertices,
            bytes: sizes.triangle_bytes,
            _pad: [0; 2],
        }
    }
}

/// The two pipelines and the buffers a batch counts into.
pub struct MeshPass {
    count: wgpu::ComputePipeline,
    count_layout: wgpu::BindGroupLayout,
    write: wgpu::ComputePipeline,
    write_layout: wgpu::BindGroupLayout,
    slots: wgpu::Buffer,
    pub(super) counts: wgpu::Buffer,
    /// Grown to the largest frame of writes seen.
    jobs: Option<wgpu::Buffer>,
    pub(super) batch: u32,
}

impl MeshPass {
    /// Room to count `batch` blocks at once.
    pub fn new(device: &wgpu::Device, atlas: &BlockAtlas, batch: u32) -> Self {
        let tiles = atlas.tiles();
        let constants: &[(&str, f64)] = &[
            ("TILES_X", f64::from(tiles.x)),
            ("TILES_Y", f64::from(tiles.y)),
        ];
        let (count, count_layout) = pipeline(device, COUNT_WGSL, "count_main", constants, 1);
        let (write, write_layout) = pipeline(device, WRITE_WGSL, "write_main", constants, 4);
        let batch = batch.max(1);
        let buffer = |label, size: u64, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            count,
            count_layout,
            write,
            write_layout,
            slots: buffer(
                "kooch_planet::mesh::slots",
                u64::from(batch) * 4,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            counts: buffer(
                "kooch_planet::mesh::counts",
                counts_bytes(batch),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            ),
            jobs: None,
            batch,
        }
    }

    /// Counts the blocks in `slots`, already generated into `atlas` earlier in the same encoder.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &BlockAtlas,
        slots: &[u32],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let slots = &slots[..slots.len().min(self.batch as usize)];
        if slots.is_empty() {
            return;
        }
        queue.write_buffer(&self.slots, 0, bytemuck::cast_slice(slots));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kooch_planet::mesh::count"),
            layout: &self.count_layout,
            entries: &[
                atlas_entry(atlas),
                entry(1, &self.slots),
                entry(2, &self.counts),
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kooch_planet::mesh::count"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.count);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(slots.len() as u32, 1, 1);
    }

    /// Writes each job's mesh into the pool's buffers and submits at once, so it reaches the
    /// queue before the next pool sync can move them.
    pub fn write(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &BlockAtlas,
        pool: &PoolBuffers,
        jobs: &[WriteJob],
    ) {
        if jobs.is_empty() {
            return;
        }
        let bytes = std::mem::size_of_val(jobs) as u64;
        if self.jobs.as_ref().is_none_or(|held| held.size() < bytes) {
            self.jobs = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("kooch_planet::mesh::jobs"),
                size: bytes.next_power_of_two(),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(held) = &self.jobs else {
            return;
        };
        queue.write_buffer(held, 0, bytemuck::cast_slice(jobs));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kooch_planet::mesh::write"),
            layout: &self.write_layout,
            entries: &[
                atlas_entry(atlas),
                entry(1, held),
                entry(2, &pool.vertices),
                entry(3, &pool.meshlet_vertices),
                entry(4, &pool.meshlet_triangles),
                entry(5, &pool.meshlets),
            ],
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kooch_planet::mesh::write"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("kooch_planet::mesh::write"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.write);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(jobs.len() as u32, 1, 1);
        }
        queue.submit([encoder.finish()]);
    }
}

/// A pipeline over `nets.wgsl` + `source`: the atlas, one read-only job buffer, then `outputs`
/// read-write buffers.
fn pipeline(
    device: &wgpu::Device,
    source: &str,
    entry_point: &str,
    constants: &[(&str, f64)],
    outputs: u32,
) -> (wgpu::ComputePipeline, wgpu::BindGroupLayout) {
    let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let mut entries = vec![
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        },
        storage(1, true),
    ];
    entries.extend((0..outputs).map(|k| storage(2 + k, false)));
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(entry_point),
        entries: &entries,
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(entry_point),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(entry_point),
        source: wgpu::ShaderSource::Wgsl(format!("{NETS_WGSL}\n{source}").into()),
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry_point),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants,
            zero_initialize_workgroup_memory: false,
        },
        cache: None,
    });
    (pipeline, layout)
}

fn atlas_entry(atlas: &BlockAtlas) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding: 0,
        resource: wgpu::BindingResource::TextureView(atlas.view()),
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

pub(super) fn counts_bytes(batch: u32) -> u64 {
    u64::from(batch) * COUNT_WORDS * 4
}
