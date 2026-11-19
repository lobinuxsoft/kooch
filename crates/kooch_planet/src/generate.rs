//! Generation (#313 step 3): the blocks the pool just took are evaluated in compute by the
//! planet's generator and written into their atlas tiles.
//!
//! The generator is WGSL defining `fn planet(input: PlanetInput) -> f32`, the signed distance in
//! metres; a `planet` graph writes one (#1178). Blocks wait in a [`Backlog`] and a frame takes a
//! bounded batch, so a jump between planets spreads its cost over frames instead of one hitch.

use std::collections::VecDeque;

use bytemuck::{Pod, Zeroable};

use super::TILE_SAMPLES;
use super::atlas::BlockAtlas;
use super::select::{bounds, voxel_size};
use super::{BlockKey, BlockPool, Planets, Residency, Shape};
use kooch_world::voxel::POOL_TEXTURE_FORMAT;

#[cfg(test)]
mod tests;

/// What a generator reads per sample. `altitude` is exact at any radius; `position` is f32 from
/// the planet's centre, so detail finer than `radius · 1e-7` is lost in it.
pub const PLANET_INPUT_WGSL: &str = r#"
struct PlanetInput {
    position: vec3<f32>,
    altitude: f32,
    direction: vec3<f32>,
    radius: f32,
    planet: u32,
}
"#;

/// The generator a planet without one gets: its base sphere, which is what an unwired `planet`
/// graph writes too.
pub const BARE_SPHERE: &str = "fn planet(input: PlanetInput) -> f32 { return input.altitude; }";

const GENERATE_WGSL: &str = include_str!("../shaders/generate.wgsl");

/// Whether `generator` compiles into the generation pass, and if not, why.
pub fn validate_generator(generator: &str) -> Result<(), String> {
    validate(&compose(generator))
}

fn compose(generator: &str) -> String {
    format!("{PLANET_INPUT_WGSL}{generator}\n{GENERATE_WGSL}")
}

fn validate(source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| e.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|e| e.emit_to_string(source))?;
    Ok(())
}

/// One block to generate. Mirrors `Job` in `planet_generate.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, PartialEq)]
pub struct Job {
    pub origin: [f32; 3],
    pub voxel: f32,
    pub radius: f32,
    pub excess: f32,
    pub slot: u32,
    pub planet: u32,
}

impl Job {
    /// `key`'s block in `slot`. The f64 terms are taken here, so the GPU never subtracts radii.
    /// `origin` is the tile's first sample, one voxel before the block's corner.
    pub fn new(key: BlockKey, slot: u32, shape: &Shape) -> Self {
        let voxel = voxel_size(shape, key.level);
        let origin = bounds(shape, key).0 - glam::DVec3::splat(voxel);
        Self {
            origin: origin.as_vec3().to_array(),
            voxel: voxel as f32,
            radius: shape.radius as f32,
            excess: (origin.length_squared() - shape.radius * shape.radius) as f32,
            slot,
            planet: key.planet,
        }
    }
}

/// Slots waiting for their block, oldest first, each once.
#[derive(Default)]
pub struct Backlog {
    queue: VecDeque<u32>,
    queued: Vec<bool>,
}

impl Backlog {
    /// Queues what the pool just allocated.
    pub fn push(&mut self, residency: &Residency) {
        for &slot in &residency.added {
            let i = slot as usize;
            if self.queued.len() <= i {
                self.queued.resize(i + 1, false);
            }
            if !self.queued[i] {
                self.queued[i] = true;
                self.queue.push_back(slot);
            }
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Queues one slot again, as for a block whose mesh did not fit its batch.
    pub fn retry(&mut self, slot: u32) {
        self.push(&Residency {
            added: vec![slot],
            removed: Vec::new(),
        });
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// The oldest slot, out of the queue.
    pub fn pop(&mut self) -> Option<u32> {
        let slot = self.queue.pop_front()?;
        if let Some(flag) = self.queued.get_mut(slot as usize) {
            *flag = false;
        }
        Some(slot)
    }

    /// Up to `max` jobs, built from what each slot holds now: a slot freed since it was queued is
    /// dropped, one reused is generated for its new block.
    pub fn take(&mut self, pool: &BlockPool, planets: &Planets, max: usize, out: &mut Vec<Job>) {
        out.clear();
        while out.len() < max {
            let Some(slot) = self.queue.pop_front() else {
                break;
            };
            if let Some(flag) = self.queued.get_mut(slot as usize) {
                *flag = false;
            }
            let Some(key) = pool.key(slot) else {
                continue;
            };
            let Some(shape) = planets.shape.get(key.planet as usize) else {
                continue;
            };
            out.push(Job::new(key, slot, shape));
        }
    }
}

/// The pipeline for one generator, and room for a batch of jobs.
pub struct GeneratePass {
    pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    jobs: wgpu::Buffer,
    batch: u32,
}

impl GeneratePass {
    /// `generator` defines `fn planet(input: PlanetInput) -> f32`; `batch` is the most blocks one
    /// [`record`](Self::record) generates. `atlas` fixes the tile layout. A generator that does not
    /// compile is an error here, never a wgpu validation panic.
    pub fn new(
        device: &wgpu::Device,
        generator: &str,
        atlas: &BlockAtlas,
        batch: u32,
    ) -> Result<Self, String> {
        let source = compose(generator);
        validate(&source)?;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kooch_planet::generate"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: POOL_TEXTURE_FORMAT,
                        view_dimension: wgpu::TextureViewDimension::D3,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kooch_planet::generate"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kooch_planet::generate"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let tiles = atlas.tiles();
        let constants: &[(&str, f64)] = &[
            ("TILES_X", f64::from(tiles.x)),
            ("TILES_Y", f64::from(tiles.y)),
            ("TILE_DIM", f64::from(TILE_SAMPLES)),
        ];
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kooch_planet::generate"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("generate_main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants,
                zero_initialize_workgroup_memory: false,
            },
            cache: None,
        });
        let batch = batch.max(1);
        let jobs = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kooch_planet::generate::jobs"),
            size: u64::from(batch) * size_of::<Job>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(Self {
            pipeline,
            layout,
            jobs,
            batch,
        })
    }

    /// The most jobs one record takes.
    pub fn batch(&self) -> u32 {
        self.batch
    }

    /// [`record`](Self::record) in an encoder of its own, submitted: for a caller with no frame to
    /// put it in.
    pub fn submit(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        atlas: &BlockAtlas,
        jobs: &[Job],
    ) {
        if jobs.is_empty() {
            return;
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kooch_planet::generate"),
        });
        self.record(device, queue, &mut encoder, atlas, jobs, None);
        queue.submit([encoder.finish()]);
    }

    /// Generates `jobs` into `atlas`; anything past [`batch`](Self::batch) is left out.
    /// `timestamps` times the pass when the device can.
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        atlas: &BlockAtlas,
        jobs: &[Job],
        timestamps: Option<wgpu::ComputePassTimestampWrites<'_>>,
    ) {
        let jobs = &jobs[..jobs.len().min(self.batch as usize)];
        if jobs.is_empty() {
            return;
        }
        queue.write_buffer(&self.jobs, 0, bytemuck::cast_slice(jobs));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kooch_planet::generate"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.jobs.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(atlas.view()),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("kooch_planet::generate"),
            timestamp_writes: timestamps,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(jobs.len() as u32, 1, 1);
    }
}
