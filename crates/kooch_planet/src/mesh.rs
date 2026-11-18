//! Meshing (#313 step 4a): surface nets over each generated block's tile, in compute, into shared
//! buffers a [`super::Terrain`] reads back a frame later. 4b writes meshlets in place instead.

use super::TILE_SAMPLES;
use super::atlas::BlockAtlas;

#[cfg(test)]
mod tests;

const MESH_WGSL: &str = include_str!("../shaders/mesh.wgsl");

/// Floats per vertex: position, then normal.
pub(super) const VERTEX_FLOATS: u64 = 6;

const _: () = assert!(
    TILE_SAMPLES == 18,
    "planet_mesh.wgsl hard-codes an 18³ tile"
);

/// The pipeline and the buffers one batch meshes into.
pub struct MeshPass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    pub(super) jobs: wgpu::Buffer,
    pub(super) header: wgpu::Buffer,
    pub(super) vertices: wgpu::Buffer,
    pub(super) indices: wgpu::Buffer,
    pub(super) batch: u32,
    pub(super) vertex_cap: u32,
    pub(super) index_cap: u32,
}

impl MeshPass {
    /// Room for `batch` blocks sharing `vertex_cap` vertices and `index_cap` indices; a block that
    /// does not fit is reported and meshed again later.
    pub fn new(
        device: &wgpu::Device,
        atlas: &BlockAtlas,
        batch: u32,
        vertex_cap: u32,
        index_cap: u32,
    ) -> Self {
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
            label: Some("kooch_planet::mesh"),
            entries: &[
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
                storage(2, false),
                storage(3, false),
                storage(4, false),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kooch_planet::mesh"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kooch_planet::mesh"),
            source: wgpu::ShaderSource::Wgsl(MESH_WGSL.into()),
        });
        let tiles = atlas.tiles();
        let constants: &[(&str, f64)] = &[
            ("TILES_X", f64::from(tiles.x)),
            ("TILES_Y", f64::from(tiles.y)),
            ("VERTEX_CAP", f64::from(vertex_cap)),
            ("INDEX_CAP", f64::from(index_cap)),
        ];
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kooch_planet::mesh"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("mesh_main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                zero_initialize_workgroup_memory: false,
            },
            cache: None,
        });
        let batch = batch.max(1);
        let buffer = |label, size: u64, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let out = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        Self {
            pipeline,
            layout,
            jobs: buffer(
                "kooch_planet::mesh::jobs",
                u64::from(batch) * 4,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            ),
            header: buffer(
                "kooch_planet::mesh::header",
                header_bytes(batch),
                out | wgpu::BufferUsages::COPY_DST,
            ),
            vertices: buffer(
                "kooch_planet::mesh::vertices",
                u64::from(vertex_cap) * VERTEX_FLOATS * 4,
                out,
            ),
            indices: buffer("kooch_planet::mesh::indices", u64::from(index_cap) * 4, out),
            batch,
            vertex_cap,
            index_cap,
        }
    }

    /// Meshes the blocks in `slots`, already generated into `atlas` earlier in the same encoder.
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
        queue.write_buffer(&self.jobs, 0, bytemuck::cast_slice(slots));
        let zeros = vec![0u8; header_bytes(self.batch) as usize];
        queue.write_buffer(&self.header, 0, &zeros);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kooch_planet::mesh"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(atlas.view()),
                },
                entry(1, &self.jobs),
                entry(2, &self.header),
                entry(3, &self.vertices),
                entry(4, &self.indices),
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kooch_planet::mesh"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(slots.len() as u32, 1, 1);
    }
}

fn entry(binding: u32, buffer: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: buffer.as_entire_binding(),
    }
}

/// Two totals, then four words per block.
pub(super) fn header_bytes(batch: u32) -> u64 {
    (2 + 4 * u64::from(batch)) * 4
}
