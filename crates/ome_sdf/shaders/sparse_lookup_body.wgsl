// Sparse SDF lookup — pure WGSL body. Defines `sparse_sdf_lookup`
// (and the `sparse_sdf_far_value` sentinel helper) reading four
// global resources by FIXED NAMES:
//
//   lookup_root_indices : storage<read>  array<u32>
//   lookup_subgrid_pool : texture_3d<f32>
//   lookup_pool_sampler : sampler
//   lookup_uniform      : uniform        LookupUniform
//
// This file deliberately does NOT declare `@group/@binding` for those
// globals. The Rust helper `crate::sparse::lookup_wgsl(group,
// root_binding, pool_binding, uniform_binding, sampler_binding)`
// prepends the four `var<...>` declarations with the caller's chosen
// slots. Same body, many host pipelines — raymarchers, the Edit
// Baker (#309), debug viz — all reuse this without forking the SDF
// math.
//
// # Caller invariant
//
// `sparse_sdf_lookup` only returns sampled SDF values for cells that
// `ClassifyPass` flagged AND `PopulatePass` filled in. Calling it
// before either pass has run on the bound `SparseGrid` is undefined —
// every cell will read as empty (`EMPTY_ROOT_SENTINEL`) and lookup
// will degrade to the `far_value` sentinel.
//
// # HW trilinear via texture atlas
//
// S6 migrated the pool to a `r16float` 3D texture atlas
// (`544 × 17 × 544`, tiles of `17³` voxels = 16³ data + 1-voxel skirt
// per face). The lookup samples through `textureSampleLevel` with a
// `Linear + ClampToEdge` sampler — the GPU's TMU does the 8-corner
// trilinear blend in one instruction, decoupled from the ALU pipe.
// The skirt voxel at each tile face stores the neighbouring root
// cell's corner sample, so the seam between two subgrids is
// reconstructed C0-continuous without a cross-tile bind dance.

struct LookupUniform {
    // `xyz` = chunk-local `bounds_min` (post-`ActiveOrigin`). `w`
    // reserved.
    bounds_min: vec4<f32>,
    // `xyz` = chunk-local `bounds_max`. `w` reserved.
    bounds_max: vec4<f32>,
}

const LOOKUP_ROOT_DIM: u32 = 16u;
const LOOKUP_SUBGRID_DIM: u32 = 16u;
const LOOKUP_TILE_DIM: u32 = 17u;
const LOOKUP_ATLAS_TILES_X: u32 = 32u;
const LOOKUP_ATLAS_DIM_X: f32 = 544.0;
const LOOKUP_ATLAS_DIM_Y: f32 = 17.0;
const LOOKUP_ATLAS_DIM_Z: f32 = 544.0;
const LOOKUP_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu;
const LOOKUP_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;

// Finite far-from-surface sentinel. Populate samples store `sdf` in
// world units; an `inf` here would poison `min`/`smooth_union` chains
// downstream. Picking `2 × max(cell_size)` keeps the value larger than
// any in-cell sample (cell_diag < 2·cell_size on any axis-aligned
// chunk) without saturating downstream f32 arithmetic.
fn sparse_sdf_far_value(bounds_min: vec3<f32>, bounds_max: vec3<f32>) -> f32 {
    let cell_size = (bounds_max - bounds_min) / f32(LOOKUP_ROOT_DIM);
    return max(max(cell_size.x, cell_size.y), cell_size.z) * 2.0;
}

// Voxel-as-corner indexing — matches `sparse_populate.wgsl`. The
// voxel at integer coords `(vx, vy, vz)` lives at world position
// `cell_min + (vx, vy, vz) / SUBGRID_DIM * cell_size`, with the skirt
// voxel `vx == 16` covering the neighbouring cell's corner. The
// fractional offset is clamped to `[0, SUBGRID_DIM]` so the texel
// coord stays inside the current tile (sampling further would hit
// the next tile in the atlas, which is unrelated in 3D space).
fn sparse_sdf_lookup(world_pos: vec3<f32>) -> f32 {
    let bounds_min = lookup_uniform.bounds_min.xyz;
    let bounds_max = lookup_uniform.bounds_max.xyz;
    let far = sparse_sdf_far_value(bounds_min, bounds_max);

    // Out-of-bounds short-circuit. `>= bounds_max` (not `>`) keeps the
    // upper face exclusive so a sample exactly at `bounds_max` does
    // not index `cell == ROOT_DIM` and walk off the root grid.
    if (any(world_pos < bounds_min) || any(world_pos >= bounds_max)) {
        return far;
    }

    let extent = bounds_max - bounds_min;
    let cell_size = extent / f32(LOOKUP_ROOT_DIM);

    // Linear → 3D root-cell index. `floor` + cast is unambiguous since
    // we already guarded against `< bounds_min` and `>= bounds_max`,
    // so `cell` is in `[0, ROOT_DIM)` on every axis.
    let local_in_root = (world_pos - bounds_min) / cell_size;
    let cell = vec3<u32>(floor(local_in_root));
    let cell_idx = cell.x
        + cell.y * LOOKUP_ROOT_DIM
        + cell.z * LOOKUP_ROOT_DIM * LOOKUP_ROOT_DIM;

    let subgrid_idx = lookup_root_indices[cell_idx];
    if (subgrid_idx == LOOKUP_EMPTY_ROOT_SENTINEL
        || subgrid_idx == LOOKUP_ALLOC_FAILED_SENTINEL) {
        return far;
    }

    let cell_min = bounds_min + vec3<f32>(cell) * cell_size;
    let local_voxel =
        (world_pos - cell_min) / cell_size * f32(LOOKUP_SUBGRID_DIM);
    // Clamp to `[0, SUBGRID_DIM]` (skirt-inclusive). Without this, a
    // sample at the cell's far face (local_voxel == SUBGRID_DIM)
    // could pick up f32 rounding into `> 16` and the sampler would
    // read into the next atlas tile — which is unrelated in 3D space
    // (atlas neighbours are not SDF neighbours).
    let local_voxel_clamped = clamp(
        local_voxel,
        vec3<f32>(0.0),
        vec3<f32>(f32(LOOKUP_SUBGRID_DIM)),
    );

    // Atlas tile origin in texel coordinates. Tiles are laid out
    // `32 × 1 × 32`; tile origin = `(idx % 32, 0, idx / 32) * 17`.
    let tile_x = subgrid_idx % LOOKUP_ATLAS_TILES_X;
    let tile_z = subgrid_idx / LOOKUP_ATLAS_TILES_X;
    let tile_origin = vec3<f32>(
        f32(tile_x * LOOKUP_TILE_DIM),
        0.0,
        f32(tile_z * LOOKUP_TILE_DIM),
    );
    let atlas_dim = vec3<f32>(LOOKUP_ATLAS_DIM_X, LOOKUP_ATLAS_DIM_Y, LOOKUP_ATLAS_DIM_Z);
    // `+ 0.5` shifts to texel centres so an integer `local_voxel`
    // (e.g. exactly at voxel `(0,0,0)`) reads that texel's stored
    // value with no fractional contribution.
    let tex_coord = (tile_origin + local_voxel_clamped + vec3<f32>(0.5)) / atlas_dim;
    return textureSampleLevel(
        lookup_subgrid_pool,
        lookup_pool_sampler,
        tex_coord,
        0.0,
    ).x;
}
