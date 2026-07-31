//! Chunk-local sparse SDF voxel storage (issue #136).
//!
//! A `SparseGrid` is the in-memory backend for one chunk's signed
//! distance field. Two-level structure (default geometry; see
//! `# Capacity` below for the `large-root-grid` feature variant):
//!
//! ```text
//! Root grid (16³ = 4096 cells)
//!   └── Subgrid (16³ = 4096 voxels each, allocated only if it
//!                contains surface)
//! ```
//!
//! Memory layout — two storage buffers, one counters buffer, and one
//! 3D texture atlas:
//!
//! - `root_indices_buffer` — `ROOT_CELLS × u32`, indexes into the
//!   subgrid pool. `EMPTY_ROOT_SENTINEL` (`0xFFFFFFFF`) = unallocated;
//!   `ALLOC_FAILED_SENTINEL` (`0xFFFFFFFE`) = pool exhausted at
//!   allocation time.
//! - `subgrid_pool_texture` — `texture_3d<r16float>` atlas of shape
//!   `(ATLAS_DIM_X, ATLAS_DIM_Y, ATLAS_DIM_Z)`. Default `(544, 17, 544)`
//!   tiled `32 × 1 × 32 = 1024` tiles. Each tile is
//!   `SUBGRID_TILE_DIM³ = 17³` voxels (16 data + 1 voxel skirt per
//!   face for HW trilinear continuity at subgrid borders). Sampled
//!   via a `LINEAR + ClampToEdge` sampler. Default total VRAM
//!   `≈ 9.6 MiB / chunk`.
//! - `free_list_buffer` — `max_subgrids × u32` stack of free pool
//!   indices; the top of the stack is tracked atomically by
//!   `counters.free_top`.
//! - `counters_buffer` — 4 × u32 atomics (`free_top`,
//!   `alloc_failed_count`, `_pad × 2`).
//!
//! # Capacity
//!
//! Default: `MAX_SUBGRIDS_DEFAULT = MAX_SUBGRIDS_PER_ATLAS = 1024`
//! matches the atlas tile capacity exactly (`32 × 1 × 32`). At
//! `<15 MB / chunk` (issue #136 AC1), the 9.6 MiB pool + small
//! bookkeeping buffers comfortably fit. Real fill is typically 5–20%
//! of the 4096 root cells per the issue body, so 1024 leaves a 25%
//! transient slack for in-flight bakes.
//!
//! With the `large-root-grid` Cargo feature (issue #347, AC4 of #136):
//! `ROOT_DIM = 32` (`32³ = 32768` cells), `ATLAS_TILES_Y = 2`,
//! `MAX_SUBGRIDS_PER_ATLAS = 2048`. Atlas LOD 0 is `(544, 34, 544)
//! r16float ≈ 19.2 MiB`; total per-chunk atlas footprint summed
//! across the 4 LODs ≈ 22.7 MiB, well inside the `<100 MiB / chunk`
//! AC4 budget. 25% headroom over the same 5% sparsity estimate
//! (32768 × 0.05 = 1638 active subgrids).
//!
//! The constructor accepts an override for chunks that warrant a
//! smaller budget (sparse open ocean, etc.) but cannot exceed the
//! atlas capacity.
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

mod chunk_lod;
mod classify;
mod downsample;
mod free_list;
mod grid;
mod lod;
mod lod_pass;
mod lookup;
mod metrics;
mod populate;
pub mod sampler;

pub use chunk_lod::{CHUNK_LOD_WGSL, ChunkLodPass, DEFAULT_LOD_DISTANCE_THRESHOLDS};
pub use classify::{CLASSIFY_WGSL, CLASSIFY_WORKGROUP_SIZE, ClassifyPass, DEFAULT_MARGIN};
pub use downsample::{
    CASCADE_COUNT, DOWNSAMPLE_WGSL, DOWNSAMPLE_WORKGROUP_SIZE, DownsamplePass,
};
pub use grid::{
    DISPATCH_INDIRECT_ARGS_SIZE, METRICS_BUFFER_SIZE, POOL_TEXTURE_FORMAT, SparseGrid,
};
pub use lod::{
    LOD_COUNT, LOD_LEVELS, LOD_VOXEL_SIZE_FACTORS, LodConfig, lod_for_voxel_size,
    lod_voxel_size,
};
pub use lod_pass::SparseLodPass;
pub use lookup::{
    LOOKUP_BODY_WGSL, LOOKUP_DEFAULT_GROUP, LOOKUP_DEFAULT_MASK_BINDING,
    LOOKUP_DEFAULT_POOL_BINDINGS, LOOKUP_DEFAULT_ROOT_BINDING,
    LOOKUP_DEFAULT_SAMPLER_BINDING, LOOKUP_DEFAULT_UNIFORM_BINDING, LookupBindings,
    lookup_wgsl,
};
pub use metrics::{METRICS_WGSL, Metrics, MetricsPass};
pub use populate::{POPULATE_WGSL, POPULATE_WORKGROUP_SIZE, PopulatePass};
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
///
/// With `large-root-grid` enabled, scales to `32` (`32³ = 32768` root
/// cells) per AC4 of #136 / issue #347.
#[cfg(not(feature = "large-root-grid"))]
pub const ROOT_DIM: u32 = 16;
/// See default-feature variant above.
#[cfg(feature = "large-root-grid")]
pub const ROOT_DIM: u32 = 32;

/// Side length (in voxels) of one subgrid's data interior. Voxels at
/// integer coords `(vx, vy, vz)` with `vx,vy,vz ∈ [0, SUBGRID_DIM)`
/// live at world position `cell_min + (vx,vy,vz) / SUBGRID_DIM *
/// cell_size`. The pool atlas reserves one extra "skirt" voxel per
/// face for HW-trilinear continuity (see `SUBGRID_TILE_DIM`).
pub const SUBGRID_DIM: u32 = 16;

/// Side length (in voxels) of one tile in the pool atlas, including
/// the 1-voxel skirt past `SUBGRID_DIM` on each face. Lookup samples
/// sit on voxel centres `[0.5, 1.5, ..., SUBGRID_TILE_DIM - 0.5]` of
/// each tile; the skirt voxel at index `SUBGRID_DIM` carries the
/// neighbouring cell's corner sample so the HW trilinear filter
/// reconstructs a C0-continuous SDF across subgrid boundaries
/// without a cross-tile bind dance.
pub const SUBGRID_TILE_DIM: u32 = 17;

/// Atlas tile counts along each axis. Default `Y = 1` because RDNA
/// 2 / 4 `texture_3d` LDS bandwidth favours wide-shallow over deep
/// stacks for this access pattern. With `large-root-grid` Y bumps to
/// 2 so the atlas holds 2048 tiles (matching the 2× root-cell count
/// at 5% sparsity headroom) without doubling the X/Z extent — that
/// would have busted the per-axis dimension budget at higher LODs.
pub const ATLAS_TILES_X: u32 = 32;
#[cfg(not(feature = "large-root-grid"))]
pub const ATLAS_TILES_Y: u32 = 1;
/// See default-feature variant above.
#[cfg(feature = "large-root-grid")]
pub const ATLAS_TILES_Y: u32 = 2;
pub const ATLAS_TILES_Z: u32 = 32;

/// Atlas dimensions in texels (`ATLAS_TILES_* × SUBGRID_TILE_DIM`).
pub const ATLAS_DIM_X: u32 = ATLAS_TILES_X * SUBGRID_TILE_DIM;
pub const ATLAS_DIM_Y: u32 = ATLAS_TILES_Y * SUBGRID_TILE_DIM;
pub const ATLAS_DIM_Z: u32 = ATLAS_TILES_Z * SUBGRID_TILE_DIM;

/// Maximum subgrids one chunk's pool atlas can hold
/// (`ATLAS_TILES_X × ATLAS_TILES_Y × ATLAS_TILES_Z = 1024`). Hard
/// upper bound on `SparseGrid::new(max_subgrids)`.
pub const MAX_SUBGRIDS_PER_ATLAS: u32 = ATLAS_TILES_X * ATLAS_TILES_Y * ATLAS_TILES_Z;

/// Total number of root cells in one `SparseGrid` (`ROOT_DIM³`).
pub const ROOT_CELLS: u32 = ROOT_DIM * ROOT_DIM * ROOT_DIM;

/// Voxels per allocated subgrid's data interior (`SUBGRID_DIM³`).
pub const SUBGRID_VOXELS: u32 = SUBGRID_DIM * SUBGRID_DIM * SUBGRID_DIM;

/// Total voxels per allocated subgrid tile in the atlas, including
/// the 1-voxel skirt per face (`SUBGRID_TILE_DIM³ = 4913`).
pub const SUBGRID_TILE_VOXELS: u32 = SUBGRID_TILE_DIM * SUBGRID_TILE_DIM * SUBGRID_TILE_DIM;

/// Default subgrid pool capacity per chunk. See module-level
/// `# Capacity` for the sizing rationale.
pub const MAX_SUBGRIDS_DEFAULT: u32 = MAX_SUBGRIDS_PER_ATLAS;

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
    MAX_SUBGRIDS_DEFAULT > 0 && MAX_SUBGRIDS_DEFAULT <= MAX_SUBGRIDS_PER_ATLAS,
    "MAX_SUBGRIDS_DEFAULT must be in 1..=MAX_SUBGRIDS_PER_ATLAS",
);
const _: () = assert!(
    MAX_SUBGRIDS_PER_ATLAS <= ROOT_CELLS,
    "atlas tile capacity must not exceed root cell count",
);

#[cfg(test)]
pub(crate) mod test_device {
    use std::sync::OnceLock;

    // Mesa radv races on parallel `request_adapter` (issue #334).
    // Acquire once per test binary and clone handles for every call.
    static SHARED: OnceLock<Option<(wgpu::Device, wgpu::Queue)>> = OnceLock::new();

    /// Acquire a wgpu device + queue for unit tests. Returns `None`
    /// when no GPU is available so the test can skip itself rather
    /// than fail (CI without a display falls into this path).
    ///
    /// Requests `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES` so the
    /// `texture_storage_3d<r16float, write>` subgrid pool atlas (S6)
    /// is usable. Adapters without that feature get treated as
    /// "no GPU" and the test skips — same behaviour CI without a
    /// display already gets.
    pub fn try_acquire() -> Option<(wgpu::Device, wgpu::Queue)> {
        SHARED
            .get_or_init(|| {
                pollster::block_on(async {
                    let instance = wgpu::Instance::default();
                    let adapter = instance
                        .request_adapter(&wgpu::RequestAdapterOptions::default())
                        .await
                        .ok()?;
                    let required = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
                    if !adapter.features().contains(required) {
                        eprintln!(
                            "skipping sparse GPU test: adapter missing {required:?}",
                        );
                        return None;
                    }
                    let (device, queue) = adapter
                        .request_device(&wgpu::DeviceDescriptor {
                            label: Some("kooch_world::voxel::test_device"),
                            required_features: required,
                            required_limits: wgpu::Limits::default(),
                            memory_hints: wgpu::MemoryHints::Performance,
                            trace: wgpu::Trace::Off,
                            experimental_features: wgpu::ExperimentalFeatures::default(),
                        })
                        .await
                        .ok()?;
                    Some((device, queue))
                })
            })
            .clone()
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
            label: Some("kooch_world::voxel::test_device::readback_staging"),
            size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("kooch_world::voxel::test_device::readback_encoder"),
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
