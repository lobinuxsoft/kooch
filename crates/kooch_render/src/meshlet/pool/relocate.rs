//! The GPU half of a repack (#773): GPU-written meshes carried from the buffers they were written
//! into to fresh ones laid out as the pool now stands.

use bytemuck::{Pod, Zeroable};

use crate::mesh::MeshVertex;
use crate::meshlet::asset::MeshletDescriptor;

use super::GlobalMeshPool;
use super::gpu::{GROWTH, GpuGlobalMeshPool, USAGE};
use super::gpu_owned::Move;
use super::slots::Span;

#[cfg(test)]
mod tests;

const MOVE_WGSL: &str = include_str!("../../../shaders/pool_move.wgsl");
/// Workgroups per dispatch row; more jobs spill into `y`.
const ROW: u32 = 65_535;

/// One move for `pool_move.wgsl`. Deltas wrap, so a mesh moving down adds a "negative" one.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Job {
    from_mv: u32,
    to_mv: u32,
    mv_len: u32,
    dv: u32,
    from_m: u32,
    to_m: u32,
    m_len: u32,
    dmv: u32,
    dt: u32,
    dm: u32,
    _pad: [u32; 2],
}

impl Job {
    fn of(m: &Move) -> Self {
        let delta = |from: Span, to: Span| to.start.wrapping_sub(from.start);
        Self {
            from_mv: m.from.meshlet_vertices.start,
            to_mv: m.to.meshlet_vertices.start,
            mv_len: m.from.meshlet_vertices.len.min(m.to.meshlet_vertices.len),
            dv: delta(m.from.vertices, m.to.vertices),
            from_m: m.from.meshlets.start,
            to_m: m.to.meshlets.start,
            m_len: m.from.meshlets.len.min(m.to.meshlets.len),
            dmv: delta(m.from.meshlet_vertices, m.to.meshlet_vertices),
            dt: delta(m.from.triangles, m.to.triangles),
            dm: delta(m.from.meshlets, m.to.meshlets),
            _pad: [0; 2],
        }
    }
}

/// The move pipeline, built on the first repack that carries a GPU-written mesh.
pub(super) struct Mover {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}

impl Mover {
    fn new(device: &wgpu::Device) -> Self {
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
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pool_move"),
            entries: &[
                storage(0, true),
                storage(1, true),
                storage(2, false),
                storage(3, true),
                storage(4, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pool_move"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pool_move"),
            source: wgpu::ShaderSource::Wgsl(MOVE_WGSL.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("pool_move"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("move_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { pipeline, layout }
    }
}

impl GpuGlobalMeshPool {
    /// Swaps the four payload buffers for ones sized to `pool`, carrying `moves` across. Submitted
    /// at once, so the writes the sync queues next land on top of it, never under it.
    pub(super) fn relocate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &GlobalMeshPool,
        moves: &[Move],
    ) {
        let sized = |len: usize, stride: usize, label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: ((len * stride) as u64).max(4).next_multiple_of(4) * GROWTH,
                usage: USAGE,
                mapped_at_creation: false,
            })
        };
        let vertex = std::mem::size_of::<MeshVertex>();
        let vertices = sized(pool.vertices.len(), vertex, "global_pool_vertices");
        let meshlet_vertices = sized(
            pool.meshlet_vertices.len(),
            4,
            "global_pool_meshlet_vertices",
        );
        let triangles = sized(
            pool.meshlet_triangles.len(),
            1,
            "global_pool_meshlet_triangles",
        );
        let meshlets = sized(
            pool.meshlets.len(),
            std::mem::size_of::<MeshletDescriptor>(),
            "global_pool_meshlets",
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("pool_move"),
        });
        for m in moves {
            let stride = vertex as u64;
            copy(
                &mut encoder,
                (&self.vertices, m.from.vertices),
                (&vertices, m.to.vertices),
                stride,
            );
            let (from, to) = (
                (&self.meshlet_triangles, m.from.triangles),
                (&triangles, m.to.triangles),
            );
            copy(&mut encoder, from, to, 1);
        }
        let jobs: Vec<Job> = moves.iter().map(Job::of).collect();
        let jobs_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pool_move_jobs"),
            size: std::mem::size_of_val(jobs.as_slice()).max(4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&jobs_buffer, 0, bytemuck::cast_slice(&jobs));
        let mover = self.mover.get_or_insert_with(|| Mover::new(device));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pool_move"),
            layout: &mover.layout,
            entries: &[
                entry(0, &jobs_buffer),
                entry(1, &self.meshlet_vertices),
                entry(2, &meshlet_vertices),
                entry(3, &self.meshlets),
                entry(4, &meshlets),
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("pool_move"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&mover.pipeline);
            pass.set_bind_group(0, &group, &[]);
            let count = jobs.len() as u32;
            pass.dispatch_workgroups(count.min(ROW), count.div_ceil(ROW), 1);
        }
        queue.submit([encoder.finish()]);
        self.vertices = vertices;
        self.meshlet_vertices = meshlet_vertices;
        self.meshlet_triangles = triangles;
        self.meshlets = meshlets;
    }
}

/// Copies a span between buffers, clipped to what the source holds: a mesh reserved after the
/// last sync has nothing written yet, and may lie past the end of the old buffer.
fn copy(
    encoder: &mut wgpu::CommandEncoder,
    (source, from): (&wgpu::Buffer, Span),
    (target, to): (&wgpu::Buffer, Span),
    stride: u64,
) {
    let at = u64::from(from.start) * stride;
    let bytes =
        (u64::from(from.len.min(to.len)) * stride).min(source.size().saturating_sub(at)) & !3;
    if bytes > 0 {
        encoder.copy_buffer_to_buffer(source, at, target, u64::from(to.start) * stride, bytes);
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}
