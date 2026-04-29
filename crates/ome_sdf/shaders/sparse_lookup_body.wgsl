// Sparse SDF lookup — pure WGSL body. Defines `sparse_sdf_lookup`
// (and the `sparse_sdf_far_value` sentinel helper) reading three
// global resources by FIXED NAMES:
//
//   lookup_root_indices : storage<read>  array<u32>
//   lookup_subgrid_pool : storage<read>  array<f32>
//   lookup_uniform      : uniform        LookupUniform
//
// This file deliberately does NOT declare `@group/@binding` for those
// globals. The Rust helper `crate::sparse::lookup_wgsl(group,
// root_binding, pool_binding, uniform_binding)` prepends the three
// `var<...>` declarations with the caller's chosen slots. Same body,
// many host pipelines — raymarchers, the Edit Baker (#309), debug viz
// — all reuse this without forking the SDF math.
//
// # Caller invariant
//
// `sparse_sdf_lookup` only returns sampled SDF values for cells that
// `ClassifyPass` flagged AND `PopulatePass` filled in. Calling it
// before either pass has run on the bound `SparseGrid` is undefined —
// every cell will read as empty (`EMPTY_ROOT_SENTINEL`) and lookup
// will degrade to the `far_value` sentinel.
//
// # C0 artifact at subgrid boundaries — intentional
//
// The trilinear filter clamps `p1` to `(15, 15, 15)` inside the
// current subgrid. Two adjacent subgrids therefore meet at a C0 (not
// C1) boundary — values agree at the seam but the gradient does not.
// This is intentional for S5: keeps the lookup self-contained (no
// cross-subgrid sampling, no neighbour bind dance) and bounds the
// raymarch error to one voxel at the seam. The fix lives downstream
// in S6 (skirt voxels per subgrid) or S7 (Vulkan hardware sampler
// over a virtual texture). Both are filed as follow-ups in the issue
// thread; the lookup ABI does not change between #136 and #309.

struct LookupUniform {
    // `xyz` = chunk-local `bounds_min` (post-`ActiveOrigin`). `w`
    // reserved.
    bounds_min: vec4<f32>,
    // `xyz` = chunk-local `bounds_max`. `w` reserved.
    bounds_max: vec4<f32>,
}

const LOOKUP_ROOT_DIM: u32 = 16u;
const LOOKUP_SUBGRID_DIM: u32 = 16u;
const LOOKUP_SUBGRID_DIM_MINUS_1: u32 = 15u;
const LOOKUP_SUBGRID_VOXELS: u32 = 4096u;
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

// Voxel-as-corner indexing — matches `sparse_populate.wgsl`. The voxel
// at integer coords `(vx, vy, vz)` lives at world position
// `cell_min + (vx, vy, vz) / SUBGRID_DIM * cell_size`. Trilinear
// reconstruction is therefore a straight 8-corner box around the
// fractional offset of `world_pos` inside the cell, with the upper
// corner clamped to `15` so a sample at the cell's far edge stays
// inside this subgrid (C0 seam — see file header).
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
    // `local_voxel` ∈ `[0, SUBGRID_DIM)` because we already established
    // `cell` is the integer floor of `local_in_root` over the same
    // cell_size — the voxel coords are the fractional remainder
    // multiplied back into the subgrid's integer grid.
    let local_voxel = (world_pos - cell_min) / cell_size * f32(LOOKUP_SUBGRID_DIM);

    let p0 = vec3<u32>(floor(local_voxel));
    let p0_clamped = min(p0, vec3<u32>(LOOKUP_SUBGRID_DIM_MINUS_1));
    let p1 = min(p0_clamped + vec3<u32>(1u), vec3<u32>(LOOKUP_SUBGRID_DIM_MINUS_1));
    let f = local_voxel - vec3<f32>(p0_clamped);

    let pool_base = subgrid_idx * LOOKUP_SUBGRID_VOXELS;
    let row = LOOKUP_SUBGRID_DIM;
    let slab = LOOKUP_SUBGRID_DIM * LOOKUP_SUBGRID_DIM;

    let s000 = lookup_subgrid_pool[pool_base + p0_clamped.x + p0_clamped.y * row + p0_clamped.z * slab];
    let s100 = lookup_subgrid_pool[pool_base + p1.x         + p0_clamped.y * row + p0_clamped.z * slab];
    let s010 = lookup_subgrid_pool[pool_base + p0_clamped.x + p1.y         * row + p0_clamped.z * slab];
    let s110 = lookup_subgrid_pool[pool_base + p1.x         + p1.y         * row + p0_clamped.z * slab];
    let s001 = lookup_subgrid_pool[pool_base + p0_clamped.x + p0_clamped.y * row + p1.z         * slab];
    let s101 = lookup_subgrid_pool[pool_base + p1.x         + p0_clamped.y * row + p1.z         * slab];
    let s011 = lookup_subgrid_pool[pool_base + p0_clamped.x + p1.y         * row + p1.z         * slab];
    let s111 = lookup_subgrid_pool[pool_base + p1.x         + p1.y         * row + p1.z         * slab];

    let c00 = mix(s000, s100, f.x);
    let c10 = mix(s010, s110, f.x);
    let c01 = mix(s001, s101, f.x);
    let c11 = mix(s011, s111, f.x);
    let c0  = mix(c00,  c10,  f.y);
    let c1  = mix(c01,  c11,  f.y);
    return mix(c0, c1, f.z);
}
