//! `OmeAccel` — the pool-backed acceleration structure. CPU-side
//! coordination + pre-allocated GPU buffers. The hot-path streaming
//! API (`insert_chunk`, `remove_chunk`, `refit_chunk`) lives in the
//! sibling [`crate::accel::streaming`] module; this module pins the
//! constructor + storage layout.

pub mod handle;

pub use handle::{ChunkBvhHandle, ChunkKey};
pub(crate) use handle::ChunkSlot;

use std::collections::HashMap;

use crate::accel::buffers::AccelBuffers;
use crate::accel::descriptor::ChunkDescriptor;
use crate::accel::error::{AccelCaps, AccelError};
use crate::accel::pool::FreeListPool;
use crate::accel::{MAX_CHUNKS_LIMIT, TLAS_REBUILD_THRESHOLD};

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

    fn make_leaf_aabb(centre_x: f32) -> crate::leaf::LeafAabb {
        crate::leaf::LeafAabb {
            aabb_min: [centre_x - 0.5, -0.5, -0.5],
            flags: crate::leaf::IS_RAYMARCH,
            aabb_max: [centre_x + 0.5, 0.5, 0.5],
            entity_id: 0,
        }
    }

    #[test]
    fn insert_chunk_round_trips_descriptor() {
        let Some((device, queue)) = skip_if_no_device() else {
            return;
        };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let leaves: Vec<_> = (0..4).map(|i| make_leaf_aabb(i as f32)).collect();
        let primitives_bytes = vec![0u8; 16 * 4];
        let handle = accel
            .insert_chunk(
                &queue,
                crate::accel::ChunkInsert {
                    key: 0xCAFE,
                    leaf_aabbs: &leaves,
                    primitives_bytes: &primitives_bytes,
                    max_smoothness_radius: 0.25,
                },
            )
            .unwrap();
        assert_eq!(accel.live_chunk_count(), 1);
        assert_eq!(accel.lookup(0xCAFE), Some(handle));
        let desc = accel.descriptor(handle).unwrap();
        // 4 leaves → 2*4 - 1 = 7 nodes total.
        assert_eq!(desc.leaf_count, 4);
        assert_eq!(desc.node_count, 7);
        assert_eq!(desc.primitive_count, 4);
        // AABB inflated by max_smoothness_radius.
        assert!(desc.aabb_min[0] <= -0.5 - 0.25 + 1e-6);
        assert!(desc.aabb_max[0] >= 3.5 + 0.25 - 1e-6);
        assert_eq!(desc.max_smoothness_radius, 0.25);
    }

    #[test]
    fn remove_chunk_frees_slots() {
        let Some((device, queue)) = skip_if_no_device() else {
            return;
        };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let leaves: Vec<_> = (0..2).map(|i| make_leaf_aabb(i as f32)).collect();
        let primitives_bytes = vec![0u8; 16 * 2];
        accel
            .insert_chunk(
                &queue,
                crate::accel::ChunkInsert {
                    key: 1,
                    leaf_aabbs: &leaves,
                    primitives_bytes: &primitives_bytes,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        let used_slots_before = AccelCaps::TEST.max_chunks - accel.free_chunk_slots.len() as u32;
        assert_eq!(used_slots_before, 1);
        accel.remove_chunk(&queue, 1).unwrap();
        assert_eq!(accel.live_chunk_count(), 0);
        assert!(accel.lookup(1).is_none());
        // The slot was returned for reuse.
        assert_eq!(accel.free_chunk_slots.len() as u32, AccelCaps::TEST.max_chunks);
        // Trying to remove twice fails cleanly.
        assert_eq!(
            accel.remove_chunk(&queue, 1),
            Err(crate::accel::AccelError::UnknownChunk)
        );
    }

    #[test]
    fn insert_two_chunks_distinct_offsets() {
        let Some((device, queue)) = skip_if_no_device() else {
            return;
        };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let leaves_a: Vec<_> = (0..3).map(|i| make_leaf_aabb(i as f32)).collect();
        let leaves_b: Vec<_> = (0..5).map(|i| make_leaf_aabb(i as f32 + 100.0)).collect();
        let prim_a = vec![0u8; 16 * 3];
        let prim_b = vec![0u8; 16 * 5];
        let h_a = accel
            .insert_chunk(
                &queue,
                crate::accel::ChunkInsert {
                    key: 1,
                    leaf_aabbs: &leaves_a,
                    primitives_bytes: &prim_a,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        let h_b = accel
            .insert_chunk(
                &queue,
                crate::accel::ChunkInsert {
                    key: 2,
                    leaf_aabbs: &leaves_b,
                    primitives_bytes: &prim_b,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        assert_ne!(h_a.chunk_idx, h_b.chunk_idx);
        let da = accel.descriptor(h_a).unwrap();
        let db = accel.descriptor(h_b).unwrap();
        // Ranges must be disjoint.
        assert!(
            da.first_node + da.node_count <= db.first_node
                || db.first_node + db.node_count <= da.first_node
        );
        assert!(
            da.first_leaf + da.leaf_count <= db.first_leaf
                || db.first_leaf + db.leaf_count <= da.first_leaf
        );
        assert!(
            da.first_primitive + da.primitive_count <= db.first_primitive
                || db.first_primitive + db.primitive_count <= da.first_primitive
        );
    }

    #[test]
    fn tlas_dirty_flips_via_update_gpu() {
        let Some((device, queue)) = skip_if_no_device() else {
            return;
        };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let leaves: Vec<_> = (0..2).map(|i| make_leaf_aabb(i as f32)).collect();
        let prim = vec![0u8; 16 * 2];
        accel
            .insert_chunk(
                &queue,
                crate::accel::ChunkInsert {
                    key: 1,
                    leaf_aabbs: &leaves,
                    primitives_bytes: &prim,
                    max_smoothness_radius: 0.0,
                },
            )
            .unwrap();
        assert!(accel.tlas_dirty_count() > 0);
        accel.update_gpu(&queue, 0.1, 0.1);
        assert_eq!(accel.tlas_dirty_count(), 0);
    }

    #[test]
    fn refit_chunk_preserves_handle() {
        let Some((device, queue)) = skip_if_no_device() else {
            return;
        };
        let mut accel = OmeAccel::new(&device, AccelCaps::TEST, 16).unwrap();
        let leaves: Vec<_> = (0..3).map(|i| make_leaf_aabb(i as f32)).collect();
        let prim = vec![0u8; 16 * 3];
        let handle = accel
            .insert_chunk(
                &queue,
                crate::accel::ChunkInsert {
                    key: 99,
                    leaf_aabbs: &leaves,
                    primitives_bytes: &prim,
                    max_smoothness_radius: 0.5,
                },
            )
            .unwrap();
        // Move every primitive +10 in x.
        let leaves_moved: Vec<_> = (0..3)
            .map(|i| make_leaf_aabb(i as f32 + 10.0))
            .collect();
        accel
            .refit_chunk(
                &queue,
                crate::accel::ChunkRefit {
                    key: 99,
                    leaf_aabbs: &leaves_moved,
                    primitives_bytes: &prim,
                    max_smoothness_radius: 0.5,
                },
            )
            .unwrap();
        let desc = accel.descriptor(handle).unwrap();
        assert!(desc.aabb_min[0] >= 9.0);
        assert!(desc.aabb_max[0] >= 12.0);
    }
}
