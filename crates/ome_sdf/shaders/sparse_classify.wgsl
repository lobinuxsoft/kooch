// Sparse classify pass — flag every root cell whose centre lies within
// one cell-diagonal of the sampled SDF surface (single-sample Lipschitz
// cone test). Run as ⌈ROOT_CELLS / 64⌉ workgroups × 64 threads = 64
// workgroups for the default 4096 root cells.
//
// `sample_sdf(p: vec3<f32>) -> f32` and `@group(1)` bindings are
// supplied by the host-prepended sampler fragment. This shader is
// opaque to the sampler implementation — it does not bind any sampler
// resources directly.
//
// # S7 — per-LOD gating
//
// The LOD index this pipeline classifies for is passed as the
// pipeline-overridable `CLASSIFY_LOD_IDX` constant. Each cell is
// gated on `chunk_lod_mask & (1 << CLASSIFY_LOD_IDX)`: if bit
// `CLASSIFY_LOD_IDX` is unset for this chunk, the workgroup
// early-returns and writes nothing. The actual cone test, the
// bookkeeping atomic, and the writes to needs_indices /
// needs_count are otherwise identical across LODs — the root grid
// resolution is LOD-independent.
//
// Output is an indirect-ready compaction so the populate pass
// (S4) can `dispatch_workgroups_indirect` over only the marked cells
// without a CPU readback in the hot loop:
//
//   classify_needs_indices[0..n] = cell_idx of each marked cell
//   classify_needs_count          = n
//
// The companion `sparse_classify_finalize.wgsl` derives the
// `[ceil_div(n, FINALIZE_WORKGROUP_SIZE), 1, 1]` indirect-args triple
// from `classify_needs_count`. Populate pins `FINALIZE_WORKGROUP_SIZE`
// to `1` (1 workgroup per marked cell); other consumers pin their
// own divisor.

const CLASSIFY_ROOT_DIM: u32 = 16u;
const CLASSIFY_ROOT_CELLS: u32 = 4096u;
const CLASSIFY_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu;
const CLASSIFY_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;

override CLASSIFY_LOD_IDX: u32 = 0u;

struct ClassifyUniform {
    // `xyz` = chunk-local bounds_min (post-`ActiveOrigin`).
    // `w`   = margin for the Lipschitz cone test:
    //         `|sample_sdf| < cell_diag * margin` → mark.
    bounds_min_margin: vec4<f32>,
    // `xyz` = chunk-local bounds_max.
    // `w`   = threshold_scale (reserved; 1.0 today).
    bounds_max_scale: vec4<f32>,
}

struct ClassifyChunkLodMask {
    value: u32,
}

@group(0) @binding(0) var<storage, read> classify_root_indices: array<u32>;
@group(0) @binding(2) var<storage, read_write> classify_needs_indices: array<u32>;
@group(0) @binding(3) var<storage, read_write> classify_needs_count: atomic<u32>;
@group(0) @binding(4) var<uniform> classify_uniform: ClassifyUniform;
@group(0) @binding(5) var<storage, read> classify_chunk_lod_mask: ClassifyChunkLodMask;

@compute @workgroup_size(64)
fn classify_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cell_idx = gid.x;
    if (cell_idx >= CLASSIFY_ROOT_CELLS) {
        return;
    }

    // S7 LOD gating: skip the entire cell if this LOD is not active for
    // the chunk. Bit `CLASSIFY_LOD_IDX` is the per-pipeline override
    // pinned by the host's `ClassifyPass::new` call.
    let lod_bit = 1u << CLASSIFY_LOD_IDX;
    if ((classify_chunk_lod_mask.value & lod_bit) == 0u) {
        return;
    }

    // Skip cells whose root entry already points to a real subgrid
    // index. `EMPTY_ROOT_SENTINEL` (unallocated) and
    // `ALLOC_FAILED_SENTINEL` (pool was exhausted last pass) both fall
    // through and get re-classified — re-running classify must be
    // idempotent against allocate, and must give failed allocations
    // another chance once the pool drains.
    let existing = classify_root_indices[cell_idx];
    if (existing < CLASSIFY_ROOT_CELLS) {
        return;
    }

    // Linear → 3D root-cell index.
    let cz = cell_idx / (CLASSIFY_ROOT_DIM * CLASSIFY_ROOT_DIM);
    let cy = (cell_idx / CLASSIFY_ROOT_DIM) % CLASSIFY_ROOT_DIM;
    let cx = cell_idx % CLASSIFY_ROOT_DIM;
    let cell_3d = vec3<f32>(f32(cx), f32(cy), f32(cz));

    let bounds_min = classify_uniform.bounds_min_margin.xyz;
    let bounds_max = classify_uniform.bounds_max_scale.xyz;
    let margin = classify_uniform.bounds_min_margin.w;

    let cell_size = (bounds_max - bounds_min) / f32(CLASSIFY_ROOT_DIM);
    let cell_diag = length(cell_size);
    let cell_center = bounds_min + (cell_3d + vec3<f32>(0.5)) * cell_size;

    let sdf = sample_sdf(cell_center);
    if (abs(sdf) < cell_diag * margin) {
        let slot = atomicAdd(&classify_needs_count, 1u);
        classify_needs_indices[slot] = cell_idx;
    }
}
