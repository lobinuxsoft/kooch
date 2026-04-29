// Sparse populate pass — for each root cell flagged by classify, pop
// a free subgrid index off the pool, sample the SDF over the subgrid's
// 16³ voxels, and write `root_indices[cell_idx] = subgrid_idx`. One
// workgroup per marked cell; threads inside the workgroup cooperate on
// the 4096 voxels of that subgrid.
//
// Concatenation order (built by `PopulatePass::new`):
//   `SPARSE_FREELIST_WGSL` (group 0 bindings 0/1 + pop helpers)
// + sampler fragment      (group 1, declares `fn sample_sdf`)
// + this file             (group 0 bindings 5..9, populate_main)
//
// Workgroup-size choice (256): 4096 voxels / 256 threads = 16 voxels
// per thread, serial inner loop. RDNA 2 (Steam Deck) wavefront 64
// → 4 waves/wg; RDNA 4 (RX 9070 XT) wavefront 32 → 8 waves/wg. Both
// occupy the SIMD well without over-subscribing register file.
//
// Indirect dispatch: `dispatch_workgroups_indirect` with x =
// `needs_count` (the populate finalize pass writes this triple). When
// the pool is exhausted mid-cell, thread 0 marks
// `root_indices[cell_idx] = ALLOC_FAILED_SENTINEL` and the whole
// workgroup early-returns — partially-written subgrid pool slots are
// not reachable through the root index, so they cause no harm and a
// later classify re-marks the cell once the pool drains.

const POPULATE_ROOT_DIM: u32 = 16u;
const POPULATE_SUBGRID_DIM: u32 = 16u;
const POPULATE_SUBGRID_VOXELS: u32 = 4096u;
const POPULATE_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;
const POPULATE_WORKGROUP_SIZE: u32 = 256u;

struct PopulateUniform {
    // `xyz` = chunk-local `bounds_min` (post-`ActiveOrigin`).
    bounds_min: vec4<f32>,
    // `xyz` = chunk-local `bounds_max`. `w` slots reserved for future
    // populate-side margin / threshold tuning without ABI churn.
    bounds_max: vec4<f32>,
}

struct PopulateNeedsCount {
    value: u32,
}

@group(0) @binding(5) var<storage, read_write> populate_root_indices: array<u32>;
@group(0) @binding(6) var<storage, read_write> populate_subgrid_pool: array<f32>;
@group(0) @binding(7) var<storage, read> populate_needs_indices: array<u32>;
@group(0) @binding(8) var<storage, read> populate_needs_count: PopulateNeedsCount;
@group(0) @binding(9) var<uniform> populate_uniform: PopulateUniform;

// Shared across the workgroup so thread 0 broadcasts the popped subgrid
// index (or the sentinel) to the other 255 threads after one barrier.
var<workgroup> wg_subgrid_idx: u32;

@compute @workgroup_size(256)
fn populate_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    // The host dispatches exactly `needs_count` workgroups, but indirect
    // args are GPU-derived — guard against a stale dispatch reading
    // beyond the compaction's filled prefix (defensive; with the
    // populate-finalize pipeline today this branch is never taken).
    if (wid.x >= populate_needs_count.value) {
        return;
    }
    let cell_idx = populate_needs_indices[wid.x];

    if (lid.x == 0u) {
        wg_subgrid_idx = sparse_pop_subgrid_index();
    }
    workgroupBarrier();
    let subgrid_idx = wg_subgrid_idx;

    if (subgrid_idx == SPARSE_ALLOC_FAILED) {
        if (lid.x == 0u) {
            populate_root_indices[cell_idx] = POPULATE_ALLOC_FAILED_SENTINEL;
        }
        return;
    }

    // Linear → 3D root-cell index (matches sparse_classify.wgsl).
    let cz = cell_idx / (POPULATE_ROOT_DIM * POPULATE_ROOT_DIM);
    let cy = (cell_idx / POPULATE_ROOT_DIM) % POPULATE_ROOT_DIM;
    let cx = cell_idx % POPULATE_ROOT_DIM;

    let bounds_min = populate_uniform.bounds_min.xyz;
    let bounds_max = populate_uniform.bounds_max.xyz;
    let cell_size = (bounds_max - bounds_min) / f32(POPULATE_ROOT_DIM);
    let cell_min_world = bounds_min
        + vec3<f32>(f32(cx), f32(cy), f32(cz)) * cell_size;

    let pool_base = subgrid_idx * POPULATE_SUBGRID_VOXELS;
    let inv_subgrid_dim = 1.0 / f32(POPULATE_SUBGRID_DIM);

    var i: u32 = lid.x;
    loop {
        if (i >= POPULATE_SUBGRID_VOXELS) {
            break;
        }
        let vz = i / (POPULATE_SUBGRID_DIM * POPULATE_SUBGRID_DIM);
        let vy = (i / POPULATE_SUBGRID_DIM) % POPULATE_SUBGRID_DIM;
        let vx = i % POPULATE_SUBGRID_DIM;
        let voxel_offset =
            vec3<f32>(f32(vx), f32(vy), f32(vz)) * inv_subgrid_dim;
        let world_pos = cell_min_world + voxel_offset * cell_size;
        populate_subgrid_pool[pool_base + i] = sample_sdf(world_pos);
        i = i + POPULATE_WORKGROUP_SIZE;
    }

    // Make the 4096 subgrid writes visible to other workgroups before
    // the root pointer publishes them. wgpu's inter-pass storage barrier
    // catches cross-pass ordering; this barrier covers the in-pass
    // happens-before edge between the cooperative voxel loop and
    // thread 0's root_indices store.
    workgroupBarrier();
    if (lid.x == 0u) {
        populate_root_indices[cell_idx] = subgrid_idx;
    }
}
