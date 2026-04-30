// Sparse populate pass — for each root cell flagged by classify, pop
// a free subgrid index off the pool, sample the SDF over the tile's
// `tile_dim³` voxels (`subgrid_dim³` data interior + 1 skirt voxel
// per face for HW trilinear continuity), and write
// `root_indices[cell_idx] = subgrid_idx`. One workgroup per marked
// cell; threads inside the workgroup cooperate on the tile voxels via
// `textureStore`.
//
// Concatenation order (built by `PopulatePass::new`):
//   `SPARSE_FREELIST_WGSL` (group 0 bindings 0/1 + pop helpers)
// + sampler fragment      (group 1, declares `fn sample_sdf`)
// + this file             (group 0 bindings 5..9, populate_main)
//
// # S7 — per-LOD override constants
//
// Atlas geometry varies per LOD. Pinning the constants below as
// pipeline overrides keeps a single shader source feeding 4
// pipelines:
//
// ```
// LOD 0: SUBGRID_DIM=16, TILE_DIM=17, TILE_VOXELS=4913, ATLAS_TILES_X=32
// LOD 1: SUBGRID_DIM= 8, TILE_DIM= 9, TILE_VOXELS= 729, ATLAS_TILES_X=32
// LOD 2: SUBGRID_DIM= 4, TILE_DIM= 5, TILE_VOXELS= 125, ATLAS_TILES_X=32
// LOD 3: SUBGRID_DIM= 2, TILE_DIM= 3, TILE_VOXELS=  27, ATLAS_TILES_X=32
// ```
//
// Workgroup-size stays at 256 across LODs. At LOD 0 each thread covers
// ~19 voxels (4913/256); at LOD 3 only ~⅒ thread does useful work
// (27/256), but the over-provisioning saves a per-LOD pipeline-shape
// check at dispatch time. Compose with the dispatch indirect args
// `[needs_count, 1, 1]` written by `populate_finalize`.
//
// # Atlas tile addressing
//
// Tiles are laid out `ATLAS_TILES_X × ATLAS_TILES_Y × ATLAS_TILES_Z`
// (default `(32, 1, 32)`; with `large-root-grid` the Y axis bumps to
// 2 so the atlas holds 2048 tiles). Tile origin for `subgrid_idx`
// follows the standard 3D index decode `(x + y · X + z · X · Y)`.
// Voxel `(vx, vy, vz)` inside the tile lives at world position
// `cell_min + (vx, vy, vz) / SUBGRID_DIM * cell_size`. The skirt
// voxel at `vx == SUBGRID_DIM` therefore evaluates the sampler at
// `cell_min + cell_size`, i.e. the corner of the neighbouring root
// cell — analytically coherent with the neighbour's own voxel(0,0,0)
// by sampler construction.

const POPULATE_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;
const POPULATE_WORKGROUP_SIZE: u32 = 256u;

override POPULATE_SUBGRID_DIM: u32 = 16u;
override POPULATE_TILE_DIM: u32 = 17u;
override POPULATE_TILE_VOXELS: u32 = 4913u;
override POPULATE_ATLAS_TILES_X: u32 = 32u;
override POPULATE_ATLAS_TILES_Y: u32 = 1u;
// Default matches the no-feature build (`ROOT_DIM = 16`).
override POPULATE_ROOT_DIM: u32 = 16u;

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
@group(0) @binding(6) var populate_subgrid_pool: texture_storage_3d<r16float, write>;
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

    // Atlas tile origin in texel coordinates. With
    // `POPULATE_ATLAS_TILES_Y == 1u` (default) the Y component
    // collapses to `0` and this matches the historical layout; with
    // `Y > 1` (large-root-grid) the second slab of tiles lives at
    // `tile_y == 1`.
    let tile_x = subgrid_idx % POPULATE_ATLAS_TILES_X;
    let tile_y = (subgrid_idx / POPULATE_ATLAS_TILES_X) % POPULATE_ATLAS_TILES_Y;
    let tile_z = subgrid_idx / (POPULATE_ATLAS_TILES_X * POPULATE_ATLAS_TILES_Y);
    let tile_origin = vec3<i32>(
        i32(tile_x * POPULATE_TILE_DIM),
        i32(tile_y * POPULATE_TILE_DIM),
        i32(tile_z * POPULATE_TILE_DIM),
    );
    let inv_subgrid_dim = 1.0 / f32(POPULATE_SUBGRID_DIM);

    var i: u32 = lid.x;
    loop {
        if (i >= POPULATE_TILE_VOXELS) {
            break;
        }
        let vz = i / (POPULATE_TILE_DIM * POPULATE_TILE_DIM);
        let vy = (i / POPULATE_TILE_DIM) % POPULATE_TILE_DIM;
        let vx = i % POPULATE_TILE_DIM;
        // Voxel offset divides by SUBGRID_DIM, NOT TILE_DIM:
        // the skirt voxel at vx == SUBGRID_DIM lives at
        // cell_min + cell_size, i.e. the next cell's corner — exactly
        // the sample needed for C0-continuous trilinear at the subgrid
        // boundary.
        let voxel_offset = vec3<f32>(f32(vx), f32(vy), f32(vz)) * inv_subgrid_dim;
        let world_pos = cell_min_world + voxel_offset * cell_size;
        let texel = tile_origin + vec3<i32>(i32(vx), i32(vy), i32(vz));
        textureStore(
            populate_subgrid_pool,
            texel,
            vec4<f32>(sample_sdf(world_pos), 0.0, 0.0, 0.0),
        );
        i = i + POPULATE_WORKGROUP_SIZE;
    }

    // Make the tile writes visible before the root pointer publishes
    // them. wgpu's inter-pass storage barrier covers cross-pass
    // ordering; this barrier covers the in-pass happens-before edge
    // between the cooperative voxel loop and thread 0's root_indices
    // store.
    workgroupBarrier();
    if (lid.x == 0u) {
        populate_root_indices[cell_idx] = subgrid_idx;
    }
}
