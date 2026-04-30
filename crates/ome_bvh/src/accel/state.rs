//! `OmeAccel` — the pool-backed acceleration structure. CPU-side
//! coordination + pre-allocated GPU buffers. The hot-path streaming
//! API (`insert_chunk`, `remove_chunk`, `refit_chunk`) lands in the
//! follow-up commit; this module pins the constructor + storage
//! layout.

use std::collections::HashMap;

use crate::accel::buffers::AccelBuffers;
use crate::accel::descriptor::ChunkDescriptor;
use crate::accel::error::{AccelCaps, AccelError};
use crate::accel::pool::FreeListPool;
use crate::accel::{MAX_CHUNKS_LIMIT, TLAS_REBUILD_THRESHOLD};

/// Opaque identifier for a chunk currently resident in the pool.
/// Returned by `insert_chunk` and consumed by `remove_chunk` /
/// `refit_chunk`. Callers that key by world-space coordinates encode
/// to a [`ChunkKey`] before insertion (a `u64` is sufficient for
/// signed `i20` × 3 axes — ~16 km radius at 16 m chunks).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ChunkBvhHandle {
    pub chunk_idx: u32,
}

/// CPU-side stable key the streaming layer uses to identify a chunk.
/// Encoding is the caller's responsibility — the pool only requires
/// `Hash + Eq + Copy`. A `u64` covers planet-scale signed `i20` × 3
/// axes with ~16 bits to spare for a generation counter.
pub type ChunkKey = u64;

/// Entry in the CPU side of the pool. Mirrors
/// `chunk_descriptors[chunk_idx]` plus the streaming bookkeeping the
/// GPU never sees. `key` is consumed by `remove_chunk` (added in the
/// streaming-API commit).
#[derive(Copy, Clone, Debug, Default)]
#[allow(dead_code)]
pub(crate) struct ChunkSlot {
    pub(crate) descriptor: ChunkDescriptor,
    pub(crate) live: bool,
    pub(crate) key: ChunkKey,
}

/// TLAS+BLAS pool acceleration structure. One instance per scene.
///
/// Owns six GPU buffers (see [`AccelBuffers`]) plus the CPU-only
/// allocators that decide where each chunk's slice lives in the
/// pools. Hot-path operations (`insert_chunk`, `remove_chunk`,
/// `refit_chunk`) thread through CPU coordination only — the GPU
/// buffers are written via `queue.write_buffer` slice writes; never
/// reallocated.
#[allow(dead_code)]
pub struct OmeAccel {
    pub(crate) caps: AccelCaps,
    pub(crate) primitive_stride: u32,

    /// Per-chunk CPU mirror. Indexed by `chunk_idx`. `live = false`
    /// marks an evicted slot waiting for the lazy TLAS compactor.
    pub(crate) slots: Vec<ChunkSlot>,
    /// Stack of free `chunk_idx` values returned by `remove_chunk`.
    pub(crate) free_chunk_slots: Vec<u32>,
    /// `key -> chunk_idx`. CPU-only — never crosses to GPU.
    pub(crate) coord_to_idx: HashMap<ChunkKey, u32>,

    pub(crate) free_node_ranges: FreeListPool,
    pub(crate) free_leaf_ranges: FreeListPool,
    pub(crate) free_primitive_ranges: FreeListPool,

    /// Counter of `inserts + removes` since the last full TLAS
    /// rebuild. Drives the lazy compactor.
    pub(crate) tlas_dirty_count: u32,

    pub buffers: AccelBuffers,
}

impl OmeAccel {
    /// Build an empty pool. All six GPU buffers are pre-allocated to
    /// the configured caps; the hot path never grows them.
    ///
    /// `primitive_stride` is the byte stride of one primitive in the
    /// `primitives_pool` (e.g. 64 for `SdfPrimitive`). Lives in
    /// constructor parameters rather than `AccelCaps` so the pool
    /// stays decoupled from the consumer crate's primitive struct.
    pub fn new(
        device: &wgpu::Device,
        caps: AccelCaps,
        primitive_stride: u32,
    ) -> Result<Self, AccelError> {
        if caps.max_chunks == 0 || caps.max_chunks > MAX_CHUNKS_LIMIT {
            return Err(AccelError::OutOfChunkSlots);
        }
        debug_assert!(primitive_stride >= 4 && primitive_stride % 4 == 0);

        let buffers = AccelBuffers::new(
            device,
            caps.max_chunks,
            caps.max_nodes,
            caps.max_leaves,
            caps.max_primitives,
            primitive_stride,
        );

        let mut slots = Vec::with_capacity(caps.max_chunks as usize);
        slots.resize(caps.max_chunks as usize, ChunkSlot::default());

        let mut free_chunk_slots = Vec::with_capacity(caps.max_chunks as usize);
        // Pop low-index slots first → byte-identical TLAS topology
        // for two scenes inserted in the same order on the same caps
        // (AC6 determinism requirement).
        for i in (0..caps.max_chunks).rev() {
            free_chunk_slots.push(i);
        }

        Ok(Self {
            caps,
            primitive_stride,
            slots,
            free_chunk_slots,
            coord_to_idx: HashMap::new(),
            free_node_ranges: FreeListPool::new(caps.max_nodes),
            free_leaf_ranges: FreeListPool::new(caps.max_leaves),
            free_primitive_ranges: FreeListPool::new(caps.max_primitives),
            tlas_dirty_count: 0,
            buffers,
        })
    }

    pub fn caps(&self) -> AccelCaps {
        self.caps
    }

    pub fn primitive_stride(&self) -> u32 {
        self.primitive_stride
    }

    /// Number of chunks currently resident in the pool (live slots
    /// only — dead-skip slots awaiting compaction excluded).
    pub fn live_chunk_count(&self) -> u32 {
        self.slots.iter().filter(|s| s.live).count() as u32
    }

    /// `inserts + removes` since the last full TLAS rebuild. The
    /// streaming layer compares against [`TLAS_REBUILD_THRESHOLD`] to
    /// decide between incremental refit and full rebuild.
    pub fn tlas_dirty_count(&self) -> u32 {
        self.tlas_dirty_count
    }

    /// Advisory: should the next `update` step do a full TLAS
    /// rebuild? Wraps the threshold comparison so call sites never
    /// drift from the canonical policy.
    pub fn tlas_should_rebuild(&self) -> bool {
        self.tlas_dirty_count >= TLAS_REBUILD_THRESHOLD
    }

    /// Look up a chunk's `chunk_idx` from its [`ChunkKey`].
    pub fn lookup(&self, key: ChunkKey) -> Option<ChunkBvhHandle> {
        self.coord_to_idx
            .get(&key)
            .copied()
            .map(|chunk_idx| ChunkBvhHandle { chunk_idx })
    }

    /// Read-only access to a resident chunk's GPU descriptor (CPU
    /// mirror).
    pub fn descriptor(&self, handle: ChunkBvhHandle) -> Option<&ChunkDescriptor> {
        self.slots
            .get(handle.chunk_idx as usize)
            .filter(|s| s.live)
            .map(|s| &s.descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_if_no_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(
            instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
        )
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("ome_accel::tests"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
            experimental_features: wgpu::ExperimentalFeatures::default(),
        }))
        .ok()?;
        Some((device, queue))
    }

    #[test]
    fn new_pre_allocates_six_buffers() {
        let Some((device, _queue)) = skip_if_no_device() else {
            return;
        };
        let accel = OmeAccel::new(&device, AccelCaps::TEST, 64).unwrap();
        assert_eq!(accel.live_chunk_count(), 0);
        assert_eq!(accel.tlas_dirty_count(), 0);
        // Free chunk stack covers every slot in low-first pop order.
        assert_eq!(accel.free_chunk_slots.len(), AccelCaps::TEST.max_chunks as usize);
        assert_eq!(*accel.free_chunk_slots.last().unwrap(), 0);
        assert_eq!(*accel.free_chunk_slots.first().unwrap(), AccelCaps::TEST.max_chunks - 1);
    }

    #[test]
    fn new_rejects_excessive_max_chunks() {
        let Some((device, _queue)) = skip_if_no_device() else {
            return;
        };
        let mut caps = AccelCaps::TEST;
        caps.max_chunks = MAX_CHUNKS_LIMIT + 1;
        assert_eq!(
            OmeAccel::new(&device, caps, 64).err(),
            Some(AccelError::OutOfChunkSlots)
        );
    }

    #[test]
    fn lookup_misses_when_empty() {
        let Some((device, _queue)) = skip_if_no_device() else {
            return;
        };
        let accel = OmeAccel::new(&device, AccelCaps::TEST, 64).unwrap();
        assert!(accel.lookup(0xDEADBEEF).is_none());
    }
}
