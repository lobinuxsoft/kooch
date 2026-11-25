//! The planet terrain's GPU side (#313): the shared atlas, generation and meshing of the blocks
//! the pool just took. What each mesh takes of the meshlet pool is read back a frame or two later
//! through a ring, so the CPU never waits on the GPU; the mesh itself never leaves it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use glam::DVec3;
use kooch_render::meshlet::{GpuMeshSizes, PoolBuffers};

use super::atlas::BlockAtlas;
use super::generate::{BARE_SPHERE, Backlog, GeneratePass, Job};
use super::mesh::{
    ALL_PARTS, COUNT_WORDS, CountJob, Cuts, MeshPass, PARTS, Part, WRITE_SHARED_BYTES, WriteJob,
    counts_bytes, sizes_of,
};
use super::recent::{BatchMean, Recent};
use super::seam::{OWN, Stale, near, touching};
use readback::Readback;

mod readback;
use super::select::{bounds, root_level, voxel_size};
use super::{BlockKey, BlockPool, Leaves, Near, Planets, Residency};

/// Batches in flight at once; a fourth waits for the first to come back.
const RING: usize = 3;

/// Blocks generated and meshed per batch.
pub const MESH_BATCH: u32 = 128;

/// Slots checked for changed neighbours per update, while anything changes.
const SWEEP: u32 = 256;

/// Generate, count and write, a timestamp either side of each. The write follows the count's
/// readback, so it is resolved with the next batch.
const QUERIES: u32 = 6;

const MAPPING: u8 = 0;
const MAPPED: u8 = 1;
const LOST: u8 = 2;

/// One block whose mesh was counted: what to reserve for it in the meshlet pool, and what the
/// write pass needs to fill it.
#[derive(Clone, Copy, Debug)]
pub struct CountedBlock {
    pub slot: u32,
    pub key: BlockKey,
    /// The block's corner from the planet's centre.
    pub origin: DVec3,
    /// Metres per voxel at the block's level.
    pub voxel: f32,
    /// Its planet's revision when it was generated; see [`Terrain::revision`].
    pub revision: u32,
    pub sizes: GpuMeshSizes,
    /// The neighbours it was counted with, which its write must use too.
    pub near: Near,
    pub cuts: Cuts,
    pub part: Part,
    /// The pool change it answers; see [`Terrain::frontier`].
    pub epoch: u64,
}

/// What the last batch cost the GPU, and the mean of the last [`RECENT`](crate::RECENT) batches: one batch
/// swings with where the camera is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuTimes {
    pub generate_ms: f32,
    /// The count pass.
    pub mesh_ms: f32,
    /// The write pass of the batch before, which may have held other blocks.
    pub write_ms: f32,
    pub blocks: u32,
    pub mean: BatchMean,
}

/// One block of a batch, as it was sent.
struct Sent {
    slot: u32,
    key: BlockKey,
    /// Its planet's revision.
    revision: u32,
    near: Near,
    part: Part,
    epoch: u64,
}

struct Flight {
    staging: wgpu::Buffer,
    blocks: Vec<Sent>,
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
    counting: Vec<CountJob>,
    /// Per slot, the block and revision its tile was last generated for.
    generated: Vec<Option<(BlockKey, u32)>>,
    stale: Stale,
    /// Seams to count again, with the epoch each was queued for.
    again: Vec<(u32, u64)>,
    /// Bumped per pool change. Everything one change asks for, the neighbours' seams included,
    /// carries its epoch, so the change goes on screen whole (#1999).
    epoch: u64,
    /// Per slot, the earliest epoch it is queued for.
    tag: Vec<u64>,
    found: Vec<u32>,
    flights: Vec<Flight>,
    timer: Option<(wgpu::QuerySet, wgpu::Buffer)>,
    period_ns: f32,
    times: Option<GpuTimes>,
    recent: Recent,
    meshed: u64,
}

impl Terrain {
    /// An atlas of `capacity` blocks. Every planet starts as its bare sphere; see
    /// [`set_generator`](Self::set_generator).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, capacity: u32) -> Result<Self, String> {
        let shared = device.limits().max_compute_workgroup_storage_size;
        if shared < WRITE_SHARED_BYTES {
            return Err(format!(
                "planet meshing needs {WRITE_SHARED_BYTES} bytes of workgroup memory, the device \
                 gives {shared}"
            ));
        }
        let atlas = BlockAtlas::new(device, capacity);
        let bare = GeneratePass::new(device, BARE_SPHERE, &atlas, MESH_BATCH)?;
        // A generated block counts all its meshes.
        let mesh = MeshPass::new(device, &atlas, MESH_BATCH * PARTS as u32);
        let size = counts_bytes(mesh.batch) + u64::from(QUERIES) * 8;
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
        let generated = vec![None; atlas.capacity() as usize];
        Ok(Self {
            atlas,
            sources: vec![BARE_SPHERE.to_owned()],
            passes: vec![bare],
            planet_pass: Vec::new(),
            revision: Vec::new(),
            mesh,
            backlog: Backlog::default(),
            generated,
            stale: Stale::default(),
            again: Vec::new(),
            epoch: 0,
            tag: Vec::new(),
            found: Vec::new(),
            jobs: Vec::new(),
            counting: Vec::new(),
            flights,
            timer,
            period_ns: queue.get_timestamp_period(),
            times: None,
            recent: Recent::default(),
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

    /// Queues what the pool just took, hands back the counts that came back, and sends the next
    /// batch if a staging buffer is free.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        leaves: Leaves,
        planets: &Planets,
        residency: &Residency,
        out: &mut Vec<CountedBlock>,
    ) {
        out.clear();
        if !residency.added.is_empty() || !residency.removed.is_empty() {
            self.epoch += 1;
            self.stale.touch(leaves.pool);
        }
        for &slot in &residency.added {
            self.mark(slot, self.epoch);
        }
        self.backlog.push(residency);
        // A block that left changes its neighbours' seams now; one that came, once generated.
        for &key in &residency.gone {
            let root = planets.shape.get(key.planet as usize).map_or(0, root_level);
            for slot in touching(leaves.pool, root, key) {
                self.queue_seam(slot, self.epoch);
            }
        }
        let (generated, revisions, pool) = (&self.generated, &self.revision, leaves.pool);
        let ready = |slot| is_ready(generated, revisions, pool, slot);
        let mut found = std::mem::take(&mut self.found);
        found.clear();
        self.stale.sweep(leaves, planets, ready, SWEEP, &mut found);
        for &slot in &found {
            self.queue_seam(slot, self.epoch);
        }
        self.found = found;
        let _ = device.poll(wgpu::PollType::Poll);
        for at in 0..self.flights.len() {
            self.collect(at, leaves.pool, planets, out);
        }
        if let Some(free) = self.flights.iter().position(|flight| !flight.busy) {
            self.send(device, queue, free, leaves, planets);
        }
    }

    /// Queues a resident block's `part` to be meshed again for `epoch`. The seam meshes from the
    /// tile it holds; the interior, which is only ever meshed with its tile, is generated again.
    pub(crate) fn remesh(&mut self, slot: u32, part: Part, epoch: u64) {
        self.mark(slot, epoch);
        match part {
            Part::Seam => self.stale.queue(slot),
            Part::Interior | Part::Overlap => self.backlog.retry(slot),
        }
    }

    fn queue_seam(&mut self, slot: u32, epoch: u64) {
        self.mark(slot, epoch);
        self.stale.queue(slot);
    }

    /// Tags `slot` with `epoch` before it is queued, keeping the earlier one while it waits.
    fn mark(&mut self, slot: u32, epoch: u64) {
        let i = slot as usize;
        if self.tag.len() <= i {
            self.tag.resize(i + 1, 0);
        }
        let waiting = self.backlog.holds(slot) || self.stale.holds(slot);
        self.tag[i] = match waiting {
            true => self.tag[i].min(epoch),
            false => epoch,
        };
    }

    fn tag_of(&self, slot: u32) -> u64 {
        self.tag.get(slot as usize).copied().unwrap_or(self.epoch)
    }

    /// The earliest epoch with work queued or in flight here; `u64::MAX` with none. Every change
    /// of an earlier epoch has been counted.
    pub(crate) fn frontier(&self) -> u64 {
        let queued = self.backlog.slots().chain(self.stale.slots());
        let flying = self
            .flights
            .iter()
            .filter(|f| f.busy)
            .flat_map(|f| &f.blocks);
        queued
            .map(|slot| self.tag_of(slot))
            .chain(flying.map(|sent| sent.epoch))
            .min()
            .unwrap_or(u64::MAX)
    }

    /// Fills granted pool ranges with the meshes of blocks counted earlier. Blocks must still hold
    /// their slot at the revision they were counted at, or their tile is no longer their mesh.
    pub fn write(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pool: &PoolBuffers,
        jobs: &[WriteJob],
    ) {
        let stamps = self
            .timer
            .as_ref()
            .map(|(set, _)| wgpu::ComputePassTimestampWrites {
                query_set: set,
                beginning_of_pass_write_index: Some(4),
                end_of_pass_write_index: Some(5),
            });
        self.mesh
            .write(device, queue, &self.atlas, pool, jobs, stamps);
    }

    /// A count bumped whenever `planet`'s generator or shape changes.
    pub fn revision(&self, planet: u32) -> u32 {
        revision_of(&self.revision, planet)
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
        leaves: Leaves,
        planets: &Planets,
    ) {
        let pool = leaves.pool;
        self.backlog
            .take(pool, planets, MESH_BATCH as usize, &mut self.jobs);
        self.again.clear();
        while self.jobs.len() + self.again.len() < MESH_BATCH as usize {
            let Some(slot) = self.stale.pop() else {
                break;
            };
            if !self.jobs.iter().any(|job| job.slot == slot)
                && !self.again.iter().any(|&(held, _)| held == slot)
            {
                self.again.push((slot, self.tag_of(slot)));
            }
        }
        if self.jobs.is_empty() && self.again.is_empty() {
            return;
        }
        // Grouped by generator: one dispatch per pipeline, in the order the mesh pass reads them.
        let passes = std::mem::take(&mut self.planet_pass);
        let pass_of = |job: &Job| passes.get(job.planet as usize).copied().unwrap_or(0);
        self.jobs.sort_by_key(pass_of);
        self.planet_pass = passes;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kooch_planet::terrain"),
        });
        // A middle group times nothing, and wgpu rejects writes with neither index.
        let writes = |first: u32, begins: bool, ends: bool| {
            self.timer
                .as_ref()
                .filter(|_| begins || ends)
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
        // Generated before the mesh pass reads it, in this encoder: a neighbour in this batch is ready.
        let mut blocks = std::mem::take(&mut self.flights[at].blocks);
        blocks.clear();
        let mut touched = Vec::new();
        for j in 0..self.jobs.len() {
            let slot = self.jobs[j].slot;
            if let Some(key) = pool.key(slot) {
                let revision = revision_of(&self.revision, key.planet);
                let epoch = self.tag_of(slot);
                for part in ALL_PARTS {
                    blocks.push(Sent {
                        slot,
                        key,
                        revision,
                        near: Near::default(),
                        part,
                        epoch,
                    });
                }
                // Its tile is in: the neighbours' seams can read it now, for the same change.
                let root = planets.shape.get(key.planet as usize).map_or(0, root_level);
                touched.extend(touching(pool, root, key).into_iter().map(|n| (n, epoch)));
                if let Some(held) = self.generated.get_mut(slot as usize) {
                    *held = Some((key, revision));
                }
            }
        }
        if !self.jobs.is_empty() {
            self.stale.touch(pool);
        }
        let (generated, revisions) = (&self.generated, &self.revision);
        let ready = |slot: u32| is_ready(generated, revisions, pool, slot);
        // Only a block whose tile is current meshes from it; one queued since is generated anyway.
        // A neighbour's change reaches only the seam: the interior reads none.
        for &(slot, epoch) in &self.again {
            if let Some(key) = pool.key(slot).filter(|_| ready(slot)) {
                let revision = revision_of(revisions, key.planet);
                blocks.push(Sent {
                    slot,
                    key,
                    revision,
                    near: Near::default(),
                    part: Part::Seam,
                    epoch,
                });
            }
        }
        self.counting.clear();
        for sent in &mut blocks {
            let root = planets
                .shape
                .get(sent.key.planet as usize)
                .map_or(0, root_level);
            // The overlap reads only its own tile, as if every side were its own.
            sent.near = match sent.part {
                Part::Overlap => Near {
                    cell: sent.key.cell.to_array(),
                    sides: [OWN; 26],
                },
                _ => near(leaves, root, sent.key, ready),
            };
            if sent.part == Part::Seam {
                self.stale.counted(sent.slot, sent.near);
            }
            self.counting
                .push(CountJob::new(sent.slot, sent.near, sent.part));
        }
        self.mesh.record(
            device,
            queue,
            &mut encoder,
            &self.atlas,
            &self.counting,
            writes(2, true, true),
        );
        for (slot, epoch) in touched {
            self.queue_seam(slot, epoch);
        }
        let flight = &mut self.flights[at];
        flight.blocks = blocks;
        let stamps = counts_bytes(self.mesh.batch);
        encoder.copy_buffer_to_buffer(&self.mesh.counts, 0, &flight.staging, 0, stamps);
        if let Some((set, resolve)) = &self.timer {
            encoder.resolve_query_set(set, 0..QUERIES, resolve, 0);
            encoder.copy_buffer_to_buffer(resolve, 0, &flight.staging, stamps, resolve.size());
        }
        queue.submit([encoder.finish()]);
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
        out: &mut Vec<CountedBlock>,
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
            for sent in blocks {
                self.backlog.retry(sent.slot);
            }
            return;
        };
        let read = Readback {
            bytes: &bytes,
            stamps: counts_bytes(self.mesh.batch),
        };
        if self.timer.is_some() {
            let mut times = read.times(self.period_ns, blocks.len() as u32);
            times.mean = self.recent.push(times);
            self.times = Some(times);
        }
        for (b, sent) in blocks.into_iter().enumerate() {
            let Sent {
                slot,
                key,
                revision,
                near,
                part,
                epoch,
            } = sent;
            // Freed, reused or reshaped since it was sent: the backlog has it again if still wanted.
            if pool.key(slot) != Some(key) || revision_of(&self.revision, key.planet) != revision {
                continue;
            }
            let Some(shape) = planets.shape.get(key.planet as usize) else {
                continue;
            };
            match read.counts(b).zip(read.cuts(b)) {
                Some((counts, cuts)) => {
                    self.meshed += 1;
                    out.push(CountedBlock {
                        slot,
                        key,
                        origin: bounds(shape, key).0,
                        voxel: voxel_size(shape, key.level) as f32,
                        revision,
                        sizes: sizes_of(counts),
                        near,
                        cuts,
                        part,
                        epoch,
                    });
                }
                None => self.backlog.retry(slot),
            }
        }
        drop(bytes);
        flight.staging.unmap();
    }
}

/// Whether `slot`'s tile holds its block at its planet's current revision.
fn is_ready(
    generated: &[Option<(BlockKey, u32)>],
    revision: &[u32],
    pool: &BlockPool,
    slot: u32,
) -> bool {
    let held = generated.get(slot as usize).copied().flatten();
    pool.key(slot)
        .is_some_and(|key| held == Some((key, revision_of(revision, key.planet))))
}

fn revision_of(revision: &[u32], planet: u32) -> u32 {
    revision.get(planet as usize).copied().unwrap_or(0)
}
