//! The planet terrain's GPU side (#313): the shared atlas, generation and meshing of the blocks
//! the pool just took, and their meshes read back a frame or two later through a ring, so the CPU
//! never waits on the GPU.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use glam::DVec3;

use super::atlas::BlockAtlas;
use super::generate::{BARE_SPHERE, Backlog, GeneratePass, Job};
use super::mesh::{MeshPass, VERTEX_FLOATS, header_bytes};
use super::select::{bounds, voxel_size};
use super::{BlockKey, BlockPool, Planets, Residency};

/// Batches in flight at once; a fourth waits for the first to come back.
const RING: usize = 3;

/// Blocks generated and meshed per batch.
pub const MESH_BATCH: u32 = 32;

const VERTEX_CAP: u32 = 65_536;
const INDEX_CAP: u32 = 393_216;

/// Two passes, a timestamp either side of each.
const QUERIES: u32 = 4;

const MAPPING: u8 = 0;
const MAPPED: u8 = 1;
const LOST: u8 = 2;

/// One block's surface, in metres from its corner, in the planet's own space.
pub struct MeshedBlock {
    pub slot: u32,
    pub key: BlockKey,
    /// The block's corner from the planet's centre.
    pub origin: DVec3,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// What the last batch cost the GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuTimes {
    pub generate_ms: f32,
    pub mesh_ms: f32,
    pub blocks: u32,
}

struct Flight {
    staging: wgpu::Buffer,
    /// Slot, block and its planet's revision when it was sent.
    blocks: Vec<(u32, BlockKey, u32)>,
    state: Arc<AtomicU8>,
    busy: bool,
}

/// Generation and meshing for every planet sharing one pool.
pub struct Terrain {
    atlas: BlockAtlas,
    /// One pipeline per distinct generator; the first is [`BARE_SPHERE`] and always stays.
    sources: Vec<String>,
    passes: Vec<GeneratePass>,
    /// Per planet: its pass, and a count bumped whenever its generator or shape changes.
    planet_pass: Vec<u32>,
    revision: Vec<u32>,
    mesh: MeshPass,
    backlog: Backlog,
    jobs: Vec<Job>,
    slots: Vec<u32>,
    flights: Vec<Flight>,
    timer: Option<(wgpu::QuerySet, wgpu::Buffer)>,
    period_ns: f32,
    times: Option<GpuTimes>,
    meshed: u64,
}

impl Terrain {
    /// An atlas of `capacity` blocks. Every planet starts as its bare sphere; see
    /// [`set_generator`](Self::set_generator).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, capacity: u32) -> Result<Self, String> {
        let atlas = BlockAtlas::new(device, capacity);
        let bare = GeneratePass::new(device, BARE_SPHERE, &atlas, MESH_BATCH)?;
        let mesh = MeshPass::new(device, &atlas, MESH_BATCH, VERTEX_CAP, INDEX_CAP);
        let size = layout(&mesh).3 + u64::from(QUERIES) * 8;
        let flights = (0..RING)
            .map(|_| Flight {
                staging: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("kooch_planet::terrain::staging"),
                    size,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                }),
                blocks: Vec::new(),
                state: Arc::new(AtomicU8::new(MAPPING)),
                busy: false,
            })
            .collect();
        let timer = device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| {
                let set = device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("kooch_planet::terrain::timer"),
                    ty: wgpu::QueryType::Timestamp,
                    count: QUERIES,
                });
                let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("kooch_planet::terrain::timer"),
                    size: u64::from(QUERIES) * 8,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                (set, resolve)
            });
        Ok(Self {
            atlas,
            sources: vec![BARE_SPHERE.to_owned()],
            passes: vec![bare],
            planet_pass: Vec::new(),
            revision: Vec::new(),
            mesh,
            backlog: Backlog::default(),
            jobs: Vec::new(),
            slots: Vec::new(),
            flights,
            timer,
            period_ns: queue.get_timestamp_period(),
            times: None,
            meshed: 0,
        })
    }

    /// Generates `planet` with `source` from now on; call it too when the planet's shape changes.
    /// Its resident blocks are queued again and what is in flight for it is dropped, so no mesh of
    /// the old shape is drawn. A source that does not compile leaves the bare sphere and says why.
    pub fn set_generator(
        &mut self,
        device: &wgpu::Device,
        pool: &BlockPool,
        planet: u32,
        source: &str,
    ) -> Result<(), String> {
        let p = planet as usize;
        if self.planet_pass.len() <= p {
            self.planet_pass.resize(p + 1, 0);
            self.revision.resize(p + 1, 0);
        }
        self.revision[p] = self.revision[p].wrapping_add(1);
        let result = match self.sources.iter().position(|known| known == source) {
            Some(pass) => {
                self.planet_pass[p] = pass as u32;
                Ok(())
            }
            None => match GeneratePass::new(device, source, &self.atlas, MESH_BATCH) {
                Ok(pass) => {
                    self.sources.push(source.to_owned());
                    self.passes.push(pass);
                    self.planet_pass[p] = (self.passes.len() - 1) as u32;
                    Ok(())
                }
                Err(why) => {
                    self.planet_pass[p] = 0;
                    Err(why)
                }
            },
        };
        self.prune();
        for slot in pool.slots_of(planet) {
            self.backlog.retry(slot);
        }
        result
    }

    /// Drops the pipelines no planet uses, so editing a generator does not pile them up.
    fn prune(&mut self) {
        let mut kept = vec![false; self.passes.len()];
        kept[0] = true;
        for &pass in &self.planet_pass {
            kept[pass as usize] = true;
        }
        let mut remap = vec![0u32; self.passes.len()];
        let mut next = 0u32;
        for (old, keep) in kept.iter().enumerate() {
            remap[old] = next;
            next += u32::from(*keep);
        }
        let sources = std::mem::take(&mut self.sources);
        let passes = std::mem::take(&mut self.passes);
        for ((source, pass), keep) in sources.into_iter().zip(passes).zip(&kept) {
            if *keep {
                self.sources.push(source);
                self.passes.push(pass);
            }
        }
        for pass in &mut self.planet_pass {
            *pass = remap[*pass as usize];
        }
    }

    fn pass_of(&self, planet: u32) -> u32 {
        self.planet_pass.get(planet as usize).copied().unwrap_or(0)
    }

    /// Queues what the pool just took, hands back the meshes that came back, and sends the next
    /// batch if a staging buffer is free.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &BlockPool,
        planets: &Planets,
        residency: &Residency,
        out: &mut Vec<MeshedBlock>,
    ) {
        out.clear();
        self.backlog.push(residency);
        let _ = device.poll(wgpu::PollType::Poll);
        for at in 0..self.flights.len() {
            self.collect(at, pool, planets, out);
        }
        if let Some(free) = self.flights.iter().position(|flight| !flight.busy) {
            self.send(device, queue, free, pool, planets);
        }
    }

    /// The atlas the blocks live in.
    pub fn atlas(&self) -> &BlockAtlas {
        &self.atlas
    }

    /// The last batch's GPU time, when the device has timestamps.
    pub fn times(&self) -> Option<GpuTimes> {
        self.times
    }

    /// Blocks waiting for a batch.
    pub fn waiting(&self) -> usize {
        self.backlog.len()
    }

    /// Batches sent and not back yet.
    pub fn in_flight(&self) -> usize {
        self.flights.iter().filter(|flight| flight.busy).count()
    }

    /// Blocks meshed since the start.
    pub fn meshed(&self) -> u64 {
        self.meshed
    }

    fn send(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        at: usize,
        pool: &BlockPool,
        planets: &Planets,
    ) {
        self.backlog
            .take(pool, planets, MESH_BATCH as usize, &mut self.jobs);
        if self.jobs.is_empty() {
            return;
        }
        // Grouped by generator: one dispatch per pipeline, in the order the mesh pass reads them.
        let passes = std::mem::take(&mut self.planet_pass);
        let pass_of = |job: &Job| passes.get(job.planet as usize).copied().unwrap_or(0);
        self.jobs.sort_by_key(pass_of);
        self.planet_pass = passes;
        self.slots.clear();
        self.slots.extend(self.jobs.iter().map(|job| job.slot));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kooch_planet::terrain"),
        });
        let writes = |first: u32, begins: bool, ends: bool| {
            self.timer
                .as_ref()
                .map(|(set, _)| wgpu::ComputePassTimestampWrites {
                    query_set: set,
                    beginning_of_pass_write_index: begins.then_some(first),
                    end_of_pass_write_index: ends.then_some(first + 1),
                })
        };
        let groups: Vec<std::ops::Range<usize>> = self
            .jobs
            .chunk_by(|a, b| self.pass_of(a.planet) == self.pass_of(b.planet))
            .scan(0, |start, group| {
                let range = *start..*start + group.len();
                *start = range.end;
                Some(range)
            })
            .collect();
        let last = groups.len().saturating_sub(1);
        for (g, range) in groups.into_iter().enumerate() {
            let jobs = &self.jobs[range];
            let Some(pass) = jobs
                .first()
                .and_then(|job| self.passes.get(self.pass_of(job.planet) as usize))
            else {
                continue;
            };
            pass.record(
                device,
                queue,
                &mut encoder,
                &self.atlas,
                jobs,
                writes(0, g == 0, g == last),
            );
        }
        self.mesh.record(
            device,
            queue,
            &mut encoder,
            &self.atlas,
            &self.slots,
            writes(2, true, true),
        );
        let (header, vertices, indices, stamps) = layout(&self.mesh);
        let flight = &mut self.flights[at];
        encoder.copy_buffer_to_buffer(&self.mesh.header, 0, &flight.staging, 0, header);
        encoder.copy_buffer_to_buffer(
            &self.mesh.vertices,
            0,
            &flight.staging,
            header,
            vertices - header,
        );
        encoder.copy_buffer_to_buffer(
            &self.mesh.indices,
            0,
            &flight.staging,
            vertices,
            indices - vertices,
        );
        if let Some((set, resolve)) = &self.timer {
            encoder.resolve_query_set(set, 0..QUERIES, resolve, 0);
            encoder.copy_buffer_to_buffer(resolve, 0, &flight.staging, stamps, resolve.size());
        }
        queue.submit([encoder.finish()]);
        flight.blocks.clear();
        for job in &self.jobs {
            if let Some(key) = pool.key(job.slot) {
                flight
                    .blocks
                    .push((job.slot, key, revision_of(&self.revision, key.planet)));
            }
        }
        flight.state.store(MAPPING, Ordering::Release);
        flight.busy = true;
        let state = flight.state.clone();
        flight
            .staging
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let done = if result.is_ok() { MAPPED } else { LOST };
                state.store(done, Ordering::Release);
            });
    }

    fn collect(
        &mut self,
        at: usize,
        pool: &BlockPool,
        planets: &Planets,
        out: &mut Vec<MeshedBlock>,
    ) {
        let flight = &mut self.flights[at];
        if !flight.busy {
            return;
        }
        let state = flight.state.load(Ordering::Acquire);
        if state == MAPPING {
            return;
        }
        flight.busy = false;
        let blocks = std::mem::take(&mut flight.blocks);
        let mapped = (state == MAPPED)
            .then(|| flight.staging.slice(..).get_mapped_range().ok())
            .flatten();
        let Some(bytes) = mapped else {
            for (slot, ..) in blocks {
                self.backlog.retry(slot);
            }
            return;
        };
        let read = Readback {
            bytes: &bytes,
            layout: layout(&self.mesh),
            vertex_cap: self.mesh.vertex_cap,
            index_cap: self.mesh.index_cap,
        };
        if self.timer.is_some() {
            self.times = Some(read.times(self.period_ns, blocks.len() as u32));
        }
        for (b, (slot, key, revision)) in blocks.into_iter().enumerate() {
            // Freed, reused or reshaped since it was sent: the backlog has it again if still wanted.
            if pool.key(slot) != Some(key) || revision_of(&self.revision, key.planet) != revision {
                continue;
            }
            let Some(shape) = planets.shape.get(key.planet as usize) else {
                continue;
            };
            match read.block(b, voxel_size(shape, key.level) as f32) {
                Some((positions, normals, indices)) => {
                    self.meshed += 1;
                    out.push(MeshedBlock {
                        slot,
                        key,
                        origin: bounds(shape, key).0,
                        positions,
                        normals,
                        indices,
                    });
                }
                None => self.backlog.retry(slot),
            }
        }
        drop(bytes);
        flight.staging.unmap();
    }
}

fn revision_of(revision: &[u32], planet: u32) -> u32 {
    revision.get(planet as usize).copied().unwrap_or(0)
}

/// Where each part sits in a staging buffer: header end, vertices end, indices end, timestamps.
fn layout(mesh: &MeshPass) -> (u64, u64, u64, u64) {
    let header = header_bytes(mesh.batch);
    let vertices = header + u64::from(mesh.vertex_cap) * VERTEX_FLOATS * 4;
    let indices = vertices + u64::from(mesh.index_cap) * 4;
    (header, vertices, indices, indices)
}

struct Readback<'a> {
    bytes: &'a [u8],
    layout: (u64, u64, u64, u64),
    vertex_cap: u32,
    index_cap: u32,
}

impl Readback<'_> {
    fn word(&self, at: u64) -> Option<u32> {
        let at = usize::try_from(at).ok()?;
        let raw = self.bytes.get(at..at + 4)?;
        Some(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn float(&self, at: u64) -> Option<f32> {
        self.word(at).map(f32::from_bits)
    }

    /// Block `b`'s mesh, positions scaled to metres; `None` when it did not fit its batch.
    #[allow(clippy::type_complexity)]
    fn block(&self, b: usize, voxel: f32) -> Option<(Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<u32>)> {
        let entry = (2 + 4 * b as u64) * 4;
        let first_vertex = self.word(entry)?;
        let vertex_count = self.word(entry + 4)?;
        let first_index = self.word(entry + 8)?;
        let index_count = self.word(entry + 12)?;
        if first_vertex.checked_add(vertex_count)? > self.vertex_cap
            || first_index.checked_add(index_count)? > self.index_cap
        {
            return None;
        }
        let mut positions = Vec::with_capacity(vertex_count as usize);
        let mut normals = Vec::with_capacity(vertex_count as usize);
        for v in first_vertex..first_vertex + vertex_count {
            let at = self.layout.0 + u64::from(v) * VERTEX_FLOATS * 4;
            let f = |k: u64| self.float(at + k * 4);
            positions.push([f(0)? * voxel, f(1)? * voxel, f(2)? * voxel]);
            normals.push([f(3)?, f(4)?, f(5)?]);
        }
        let mut indices = Vec::with_capacity(index_count as usize);
        for i in first_index..first_index + index_count {
            let index = self.word(self.layout.1 + u64::from(i) * 4)?;
            if index >= vertex_count {
                return None;
            }
            indices.push(index);
        }
        Some((positions, normals, indices))
    }

    fn times(&self, period_ns: f32, blocks: u32) -> GpuTimes {
        let stamp = |k: u64| {
            let lo = self.word(self.layout.3 + k * 8).unwrap_or(0);
            let hi = self.word(self.layout.3 + k * 8 + 4).unwrap_or(0);
            u64::from(lo) | (u64::from(hi) << 32)
        };
        let ms = |from: u64, to: u64| (to.saturating_sub(from)) as f32 * period_ns / 1e6;
        GpuTimes {
            generate_ms: ms(stamp(0), stamp(1)),
            mesh_ms: ms(stamp(2), stamp(3)),
            blocks,
        }
    }
}
