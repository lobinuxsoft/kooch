//! `OmeAccel` — the pool-backed acceleration structure. CPU-side
//! coordination + pre-allocated GPU buffers. The hot-path streaming
//! API (`insert_chunk`, `remove_chunk`, `refit_chunk`) lives in the
//! sibling [`crate::accel::streaming`] module; this module pins the
//! constructor + storage layout.

pub mod handle;
#[cfg(test)]
mod tests;

pub use handle::{ChunkBvhHandle, ChunkKey};
pub(crate) use handle::ChunkSlot;

use std::collections::HashMap;

use crate::accel::buffers::AccelBuffers;
use crate::accel::descriptor::ChunkDescriptor;
use crate::accel::error::{AccelCaps, AccelError};
use crate::accel::pool::FreeListPool;
use crate::accel::{MAX_CHUNKS_LIMIT, TLAS_REBUILD_THRESHOLD};
use crate::gpu::tlas_lbvh::TlasGpuBuilder;
use crate::node::BvhNode;

/// TLAS+BLAS pool acceleration structure. Owns six pre-allocated GPU
/// buffers (see [`AccelBuffers`]) plus the CPU coordinators that
/// decide where each chunk's slice lives in the pools. Hot-path
/// operations write through `queue.write_buffer` only — buffers are
/// never reallocated.
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

    /// CPU mirror of `tlas_nodes`, kept in lockstep by `tlas::rebuild`
    /// for the CPU traversal helper. Empty when the pool has no live
    /// chunks. GPU shader still reads `buffers.tlas_nodes`.
    pub(crate) cpu_tlas_nodes: Vec<BvhNode>,

    pub buffers: AccelBuffers,

    /// Persistent TLAS GPU rebuild pipelines + scratch (epic #370 PR-1).
    /// Lazily constructed on the first rebuild — the device limits
    /// required by the onesweep sort (6 storage buffers in one stage)
    /// exceed `downlevel_defaults`, so tests that never trigger a
    /// rebuild (CPU traversal, refit-only paths) keep using the
    /// minimal device profile. Once built, the builder lives for the
    /// rest of the `OmeAccel` lifetime.
    pub(crate) tlas_builder: Option<TlasGpuBuilder>,

    /// Cloned `wgpu::Device` handle (Arc-backed; cheap to clone).
    /// Cached so `tlas::rebuild` can issue bind groups against the
    /// builder without threading the device through every public
    /// API. Commit 9 will hoist the encoder to the caller; the device
    /// stays cached because the builder uses it on every dispatch.
    pub(crate) device: wgpu::Device,
}

impl OmeAccel {
    /// Build an empty pool. All six GPU buffers are pre-allocated to
    /// the configured caps; the hot path never grows them.
    ///
    /// `primitive_stride` is the byte stride of one primitive in the
    /// `primitives_pool` (e.g. 64 for `VolumePrimitive`). Lives in
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
        slots.resize_with(caps.max_chunks as usize, ChunkSlot::default);

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
            cpu_tlas_nodes: Vec::new(),
            buffers,
            tlas_builder: None,
            device: device.clone(),
        })
    }

    /// Lazily construct the TLAS GPU rebuild pipelines on first use.
    /// Idempotent — re-runs after a builder already exists are O(1).
    pub(crate) fn ensure_tlas_builder(&mut self) -> &TlasGpuBuilder {
        if self.tlas_builder.is_none() {
            self.tlas_builder = Some(TlasGpuBuilder::new(&self.device, None));
        }
        self.tlas_builder.as_ref().unwrap()
    }

    /// CPU-side fold of every live slot's `ChunkDescriptor`. Consumed
    /// by `tlas::rebuild` to compute the scene-wide `GpuSceneBounds`
    /// for the morton normalisation pass — the GPU shader reads
    /// chunks through `buffers.tlas_live_chunk_indices`, so the
    /// caller of `rebuild` only needs the CPU mirror for that fold.
    pub(crate) fn live_chunk_descriptors(&self) -> Vec<ChunkDescriptor> {
        self.slots
            .iter()
            .filter(|s| s.live)
            .map(|s| s.descriptor)
            .collect()
    }

    /// Slot indices of every live chunk, in slot order (i.e. the same
    /// order [`Self::live_chunk_descriptors`] produces). Uploaded into
    /// `buffers.tlas_live_chunk_indices` so the TLAS Karras passes
    /// (morton + leaves) can resolve their contiguous `k ∈ [0, n)`
    /// thread index back to the slot index that
    /// `buffers.chunk_descriptors` is keyed by — see the slot-vs-Morton
    /// indirection note in `shaders/tlas_morton.wgsl`.
    pub(crate) fn live_chunk_indices(&self) -> Vec<u32> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| if s.live { Some(i as u32) } else { None })
            .collect()
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

    /// Iterate `(chunk_idx, &[LeafAabb])` for every live chunk in the
    /// pool. Skips dead-slot entries waiting for the lazy compactor.
    /// Used by `ome_physics::broadphase` to enumerate every collider
    /// leaf across the pool without exposing the internal slot type.
    pub fn iter_live_leaves(&self) -> impl Iterator<Item = (u32, &[crate::leaf::LeafAabb])> {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.live)
            .map(|(idx, s)| (idx as u32, s.cpu_leaf_aabbs.as_slice()))
    }

    /// Fragmentation snapshot of the BLAS node pool — the largest of
    /// the three byte pools and the one most prone to fragmentation
    /// under churn. Used by AC7 to gate the streaming round-trip
    /// against runaway fragmentation. Free-range coalescing happens
    /// inside the call so the snapshot reflects the canonical post-
    /// coalesce shape.
    pub fn node_pool_fragmentation(&mut self) -> crate::accel::FragmentationMetrics {
        self.free_node_ranges.fragmentation_metrics()
    }
}

