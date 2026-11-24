//! Generation (#313 step 3): the blocks the pool just took are evaluated in compute by the
//! planet's generator and written into their atlas tiles.
//!
//! The generator is WGSL defining `fn planet(input: PlanetInput) -> f32`, the signed distance in
//! metres; a `planet` graph writes one (#1178). Blocks wait in a [`Backlog`] and a frame takes a
//! bounded batch, so a jump between planets spreads its cost over frames instead of one hitch.

use std::collections::VecDeque;

use bytemuck::{Pod, Zeroable};
use glam::IVec3;

use super::atlas::BlockAtlas;
use super::select::voxel_size;
use super::{BLOCK_SAMPLES, TILE_SAMPLES};
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
    validate(&compose(generator, glam::UVec3::new(32, 1, 1)))
}

/// The generate shader for `generator`, over an atlas of `tiles`.
fn compose(generator: &str, tiles: glam::UVec3) -> String {
    format!(
        "const TILES_X: u32 = {}u;\nconst TILES_Y: u32 = {}u;\nconst TILE_DIM: u32 = {TILE_SAMPLES}u;\n\
         {PLANET_INPUT_WGSL}{generator}\n{GENERATE_WGSL}",
        tiles.x, tiles.y
    )
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
    /// The tile's first sample on the planet's grid, in voxels of its level: neighbours derive a
    /// shared sample's position from the same integers, so they agree on it bit for bit (#1933).
    pub index: [i32; 3],
    pub voxel: f32,
    pub radius: f32,
    pub slot: u32,
    pub planet: u32,
    pub _pad: u32,
    /// Per [`anchors`] of the tile, `|anchor|² − radius²`.
    pub excess: [f32; 8],
}

impl Job {
    /// `key`'s block in `slot`. The f64 terms are taken here, so the GPU never subtracts radii.
    /// The tile starts one voxel before the block's corner.
    pub fn new(key: BlockKey, slot: u32, shape: &Shape) -> Self {
        let voxel = voxel_size(shape, key.level);
        let index = key.cell * BLOCK_SAMPLES as i32 - 1;
        let excess = anchors(index).map(|anchor| {
            let at = anchor.as_dvec3() * voxel;
            (at.length_squared() - shape.radius * shape.radius) as f32
        });
        Self {
            index: index.to_array(),
            voxel: voxel as f32,
            radius: shape.radius as f32,
            slot,
            planet: key.planet,
            _pad: 0,
            excess,
        }
    }
}

/// Grid period of the altitude anchors. Mirrors `ANCHOR` in `generate.wgsl`.
const ANCHOR: i32 = 32;

/// The eight anchors a tile starting at `index` meets, x fastest: the altitude of a sample is taken
/// from the one below it on the grid, never from its tile, so two tiles sharing it agree.
fn anchors(index: IVec3) -> [IVec3; 8] {
    let base = index & IVec3::splat(-ANCHOR);
    std::array::from_fn(|k| {
        let k = k as i32;
        base + IVec3::new(k & 1, (k >> 1) & 1, k >> 2) * ANCHOR
    })
}

/// Slots waiting for their block, each once; a batch takes the nearest visible first.
#[derive(Default)]
pub struct Backlog {
    queue: VecDeque<u32>,
    queued: Vec<bool>,
    ranked: Vec<(f64, u32)>,
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
            ..Residency::default()
        });
    }

    /// Whether `slot` waits in the queue.
    pub fn holds(&self, slot: u32) -> bool {
        self.queued.get(slot as usize).copied().unwrap_or(false)
    }

    /// Every slot waiting.
    pub fn slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.queue.iter().copied()
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

    /// Up to `max` jobs, the most urgent first (`sight::urgency`), built from what each slot holds
    /// now: a slot freed since it was queued is dropped, one reused is generated for its new block.
    pub fn take(&mut self, pool: &BlockPool, planets: &Planets, max: usize, out: &mut Vec<Job>) {
        out.clear();
        self.ranked.clear();
        let queued = &mut self.queued;
        self.ranked.extend(
            self.queue
                .drain(..)
                .filter_map(|slot| match pool.key(slot) {
                    Some(key) => Some((crate::sight::urgency(planets, key), slot)),
                    None => {
                        // Freed since it was queued: nothing to generate, and free to queue again.
                        if let Some(flag) = queued.get_mut(slot as usize) {
                            *flag = false;
                        }
                        None
                    }
                }),
        );
        // Ranked again every batch: the camera moves while the backlog waits.
        let cut = max.min(self.ranked.len());
        if cut < self.ranked.len() {
            self.ranked
                .select_nth_unstable_by(cut, |a, b| a.0.total_cmp(&b.0));
        }
        self.ranked[..cut].sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        self.queue
            .extend(self.ranked[cut..].iter().map(|&(_, slot)| slot));
        for i in 0..cut {
            let slot = self.ranked[i].1;
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
        let source = compose(generator, atlas.tiles());
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
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("kooch_planet::generate"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("generate_main"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[],
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
