//! The ground's samples from the GPU (#1949): the generate pass that draws a planet fills a small
//! atlas of the ground's own, and each tile is read back a frame or two later, so a collider is
//! meshed from exactly the samples that are drawn.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;

use crate::{BlockAtlas, BlockKey, GeneratePass, Job, Shape, TILE_SAMPLES};

/// Tiles in flight at once; each is a slot of the atlas and a staging buffer.
pub(super) const SLOTS: u32 = 16;

/// `bytes_per_row` of a tile row in staging: wgpu copies rows aligned to 256 bytes.
const ROW: u32 = 256;

const IDLE: u8 = 0;
const MAPPING: u8 = 1;
const MAPPED: u8 = 2;
const LOST: u8 = 3;

/// A block to generate, with what comes back with it.
pub(super) struct Order<T> {
    pub key: BlockKey,
    pub shape: Shape,
    /// The planet's generator revision, which names its compiled pass.
    pub twin: u32,
    pub source: Arc<str>,
    pub ticket: T,
}

struct Flight<T> {
    staging: wgpu::Buffer,
    state: Arc<AtomicU8>,
    ticket: Option<T>,
}

/// A device, the ground's atlas, and one compiled pass per planet generator.
pub(super) struct GroundGpu<T> {
    device: wgpu::Device,
    queue: wgpu::Queue,
    atlas: BlockAtlas,
    /// By planet and revision; `None` when the generator does not compile.
    passes: HashMap<(u32, u32), Option<GeneratePass>>,
    flights: Vec<Flight<T>>,
}

impl<T> GroundGpu<T> {
    /// The frame's device where there is one, or a compute-only device of its own: the editor's
    /// remote host simulates with no window and no GPU context.
    pub(super) fn acquire(resources: &Resources) -> Option<Self> {
        let (device, queue) = match resources.get::<GpuContext>() {
            Some(gpu) => (gpu.device().clone(), gpu.queue().clone()),
            None => headless()?,
        };
        let atlas = BlockAtlas::new(&device, SLOTS);
        let size = u64::from(ROW * TILE_SAMPLES * TILE_SAMPLES);
        let flights = (0..SLOTS)
            .map(|_| Flight {
                staging: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("kooch_planet::ground::staging"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                state: Arc::new(AtomicU8::new(IDLE)),
                ticket: None,
            })
            .collect();
        Some(Self {
            device,
            queue,
            atlas,
            passes: HashMap::new(),
            flights,
        })
    }

    /// Slots free for [`submit`](Self::submit).
    pub(super) fn free(&self) -> usize {
        self.flights.iter().filter(|f| f.ticket.is_none()).count()
    }

    /// Generates `orders` into free slots and starts reading each tile back. Orders past the free
    /// slots, or whose generator does not compile, come back from [`collect`](Self::collect) with
    /// no samples.
    pub(super) fn submit(&mut self, orders: Vec<Order<T>>) -> Vec<T> {
        let mut refused = Vec::new();
        let mut jobs: HashMap<(u32, u32), Vec<Job>> = HashMap::new();
        let mut slots = Vec::new();
        for order in orders {
            let pass = (order.key.planet, order.twin);
            if !self.passes.contains_key(&pass) {
                let compiled = GeneratePass::new(&self.device, &order.source, &self.atlas, SLOTS)
                    .inspect_err(|why| {
                        tracing::warn!(target: "kooch_planet", "ground pass does not compile: {why}");
                    })
                    .ok();
                self.passes
                    .retain(|(planet, _), _| *planet != order.key.planet);
                self.passes.insert(pass, compiled);
            }
            let free = self.flights.iter().position(|f| f.ticket.is_none());
            let (Some(slot), Some(Some(_))) = (free, self.passes.get(&pass)) else {
                refused.push(order.ticket);
                continue;
            };
            let slot = slot as u32;
            jobs.entry(pass)
                .or_default()
                .push(Job::new(order.key, slot, &order.shape));
            if let Some(flight) = self.flights.get_mut(slot as usize) {
                flight.ticket = Some(order.ticket);
            }
            slots.push(slot);
        }
        if slots.is_empty() {
            return refused;
        }
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("kooch_planet::ground"),
            });
        for (pass, jobs) in &jobs {
            if let Some(Some(pass)) = self.passes.get(pass) {
                pass.record(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &self.atlas,
                    jobs,
                    None,
                );
            }
        }
        for &slot in &slots {
            if let Some(flight) = self.flights.get(slot as usize) {
                copy_tile(&mut encoder, &self.atlas, slot, &flight.staging);
            }
        }
        self.queue.submit([encoder.finish()]);
        for &slot in &slots {
            let Some(flight) = self.flights.get(slot as usize) else {
                continue;
            };
            flight.state.store(MAPPING, Ordering::Release);
            let state = Arc::clone(&flight.state);
            flight
                .staging
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |result| {
                    let done = if result.is_ok() { MAPPED } else { LOST };
                    state.store(done, Ordering::Release);
                });
        }
        refused
    }

    /// The tiles read back since the last call, as `sdf / voxel` x fastest; `None` for one the
    /// device lost.
    pub(super) fn collect(&mut self) -> Vec<(T, Option<Vec<f32>>)> {
        let _ = self.device.poll(wgpu::PollType::Poll);
        let mut out = Vec::new();
        for flight in &mut self.flights {
            let state = flight.state.load(Ordering::Acquire);
            if flight.ticket.is_none() || !(state == MAPPED || state == LOST) {
                continue;
            }
            let samples = (state == MAPPED).then(|| decode(&flight.staging)).flatten();
            if state == MAPPED {
                flight.staging.unmap();
            }
            flight.state.store(IDLE, Ordering::Release);
            if let Some(ticket) = flight.ticket.take() {
                out.push((ticket, samples));
            }
        }
        out
    }
}

/// A device with the `r16float` storage the atlas needs, without a surface.
fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .inspect_err(|why| tracing::warn!(target: "kooch_planet", "no GPU for the ground: {why}"))
    .ok()?;
    let required = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(required) {
        tracing::warn!(target: "kooch_planet", "the GPU cannot store the ground's r16float tiles");
        return None;
    }
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("kooch_planet::ground"),
        required_features: required,
        required_limits: kooch_core::gpu::elevated_compute_limits(&adapter),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .inspect_err(
        |why| tracing::warn!(target: "kooch_planet", "no GPU device for the ground: {why}"),
    )
    .ok()
}

fn copy_tile(
    encoder: &mut wgpu::CommandEncoder,
    atlas: &BlockAtlas,
    slot: u32,
    staging: &wgpu::Buffer,
) {
    let corner = atlas.corner(slot);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: atlas.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: corner.x,
                y: corner.y,
                z: corner.z,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ROW),
                rows_per_image: Some(TILE_SAMPLES),
            },
        },
        wgpu::Extent3d {
            width: TILE_SAMPLES,
            height: TILE_SAMPLES,
            depth_or_array_layers: TILE_SAMPLES,
        },
    );
}

/// A mapped tile, x fastest, unpadded from its 256-byte rows.
fn decode(staging: &wgpu::Buffer) -> Option<Vec<f32>> {
    let dim = TILE_SAMPLES as usize;
    let bytes = staging.slice(..).get_mapped_range().ok()?;
    let mut out = Vec::with_capacity(dim * dim * dim);
    for row in 0..dim * dim {
        let start = row * ROW as usize;
        for x in 0..dim {
            let at = start + x * 2;
            let pair: [u8; 2] = bytes.get(at..at + 2)?.try_into().ok()?;
            out.push(half::f16::from_bits(u16::from_le_bytes(pair)).to_f32());
        }
    }
    Some(out)
}
