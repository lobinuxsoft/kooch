//! Chunk-local sparse SDF voxel storage (issue #136).
//!
//! A `SparseGrid` is the in-memory backend for one chunk's signed
//! distance field. Two-level structure:
//!
//! ```text
//! Root grid (16³ = 4096 cells)
//!   └── Subgrid (16³ = 4096 voxels each, allocated only if it
//!                contains surface)
//! ```
//!
//! Memory layout — three storage buffers + one counters buffer:
//!
//! - `root_indices_buffer` — `ROOT_CELLS × u32`, indexes into the
//!   subgrid pool. `EMPTY_ROOT_SENTINEL` (`0xFFFFFFFF`) = unallocated;
//!   `ALLOC_FAILED_SENTINEL` (`0xFFFFFFFE`) = pool exhausted at
//!   allocation time.
//! - `subgrid_pool_buffer` — `max_subgrids × SUBGRID_VOXELS × f32`
//!   voxel values.
//! - `free_list_buffer` — `max_subgrids × u32` stack of free pool
//!   indices; the top of the stack is tracked atomically by
//!   `counters.free_top`.
//! - `counters_buffer` — 4 × u32 atomics (`free_top`,
//!   `alloc_failed_count`, `_pad × 2`).
//!
//! # Capacity
//!
//! `MAX_SUBGRIDS_DEFAULT = 1024` matches the issue's
//! `<15 MB / chunk` acceptance criterion (1024 × 16 KB = 16 MB pool;
//! the small overshoot is power-of-two alignment headroom). Real fill
//! is typically 5–20% of the 4096 root cells per the issue body, so
//! 1024 leaves a 25% transient slack for in-flight bakes. The
//! constructor accepts an override for chunks that warrant a
//! different budget (dense caves, sparse open ocean, etc.).
//!
//! # Encoder ordering invariant
//!
//! Mutating compute passes (classify / allocate / populate / free —
//! shipped in subsequent subtasks) must precede sampling reads of the
//! same `SparseGrid` within the same `wgpu` submission. wgpu inserts
//! the implicit storage-buffer memory barrier between consecutive
//! compute passes, and queue submission order provides cross-frame
//! ordering. Multi-frame partial bakes are the consumer's
//! responsibility — see issue #309 (Edit Baker)'s atomic-per-chunk
//! submission contract.

mod classify;
mod free_list;
mod grid;
pub mod sampler;

pub use classify::{
    CLASSIFY_FINALIZE_WGSL, CLASSIFY_WGSL, CLASSIFY_WORKGROUP_SIZE, ClassifyPass, DEFAULT_MARGIN,
};
pub use grid::{DISPATCH_INDIRECT_ARGS_SIZE, SparseGrid};
pub use sampler::{ANALYTIC_SPHERE_WGSL, AnalyticSphereSampler, SdfSampler};

/// Source of `shaders/sparse_freelist.wgsl` — atomic free-list pop /
/// push helpers shared by the allocate (#S4) and free (#S7) compute
/// shaders. Consumer pipelines concat this string ahead of their own
/// shader source.
pub const SPARSE_FREELIST_WGSL: &str =
    include_str!("../../shaders/sparse_freelist.wgsl");

/// Size in bytes of the `SparseCounters` struct (mirrors the WGSL
/// layout in `sparse_freelist.wgsl`). Four `u32`s: `free_top`,
/// `alloc_failed_count`, two padding slots.
pub const FREELIST_COUNTERS_SIZE: u64 = 16;

/// Side length (in cells) of the root grid. Each chunk owns one root
/// grid, addressing `ROOT_CELLS = ROOT_DIM³` subgrid slots.
pub const ROOT_DIM: u32 = 16;

/// Side length (in voxels) of one subgrid. Subgrids are dense — every
/// allocated subgrid holds `SUBGRID_VOXELS = SUBGRID_DIM³` f32 values.
pub const SUBGRID_DIM: u32 = 16;

/// Total number of root cells in one `SparseGrid` (`ROOT_DIM³`).
pub const ROOT_CELLS: u32 = ROOT_DIM * ROOT_DIM * ROOT_DIM;

/// Voxels per allocated subgrid (`SUBGRID_DIM³`).
pub const SUBGRID_VOXELS: u32 = SUBGRID_DIM * SUBGRID_DIM * SUBGRID_DIM;

/// Default subgrid pool capacity per chunk. See module-level
/// `# Capacity` for the sizing rationale.
pub const MAX_SUBGRIDS_DEFAULT: u32 = 1024;

/// `root_indices` value meaning "no subgrid allocated for this cell".
/// The lookup shader returns `FAR_FROM_SURFACE` for empty cells so
/// `min(empty, x) ≈ x` keeps the raymarch identity-element invariant
/// (matches the `+1e10` plane sentinel from #115 PR-4).
pub const EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFF;

/// `root_indices` value meaning "allocation requested but the pool was
/// exhausted". Distinguished from `EMPTY_ROOT_SENTINEL` so diagnostics
/// can flag pool-exhaustion bugs without confusing them with regular
/// empty cells.
pub const ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFE;

/// Sentinel SDF value returned for empty / out-of-bounds samples. Same
/// magnitude the raymarch uses for its identity element so smooth
/// CSG operators degrade cleanly (`smooth_union(1e10, x, k) ≈ x`).
pub const FAR_FROM_SURFACE: f32 = 1e10;

const _: () = assert!(
    MAX_SUBGRIDS_DEFAULT > 0 && MAX_SUBGRIDS_DEFAULT <= ROOT_CELLS,
    "MAX_SUBGRIDS_DEFAULT must be in 1..=ROOT_CELLS",
);

#[cfg(test)]
pub(crate) mod test_device {
    /// Acquire a wgpu device + queue for unit tests. Returns `None`
    /// when no GPU is available so the test can skip itself rather
    /// than fail (CI without a display falls into this path).
    ///
    /// Unlike `ome_bvh::test_device`, no extra features are requested
    /// — the sparse storage subsystem only uses storage buffers and
    /// plain compute. Adapters without `TIMESTAMP_QUERY` are
    /// acceptable here; the diagnostics subtask (S8) will gate that.
    pub fn try_acquire() -> Option<(wgpu::Device, wgpu::Queue)> {
        pollster::block_on(async {
            let instance = wgpu::Instance::default();
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .ok()?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("ome_sdf::sparse::test_device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                })
                .await
                .ok()?;
            Some((device, queue))
        })
    }

    /// Synchronous full-buffer readback helper for tests. `src` must
    /// have `COPY_SRC` usage (every `SparseGrid` buffer does).
    pub fn readback(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        src: &wgpu::Buffer,
    ) -> Vec<u8> {
        let size = src.size();
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ome_sdf::sparse::test_device::readback_staging"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ome_sdf::sparse::test_device::readback_encoder"),
        });
        encoder.copy_buffer_to_buffer(src, 0, &staging, 0, size);
        queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).ok();
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .expect("device poll");
        rx.recv().expect("readback channel").expect("map_async ok");

        let view = slice.get_mapped_range();
        let bytes = view.to_vec();
        drop(view);
        staging.unmap();
        bytes
    }
}
