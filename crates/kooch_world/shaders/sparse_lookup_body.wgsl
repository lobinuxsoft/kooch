// Sparse SDF lookup — pure WGSL body, S7 LOD-aware. Defines
// `sparse_sdf_lookup(world_pos: vec3<f32>, target_voxel_size: f32) -> f32`
// reading SEVEN global resources by FIXED NAMES:
//
//   lookup_root_indices       : storage<read>  array<u32>  (canonical, see below)
//   lookup_subgrid_pool_lod0  : texture_3d<f32>            (LOD 0 atlas)
//   lookup_subgrid_pool_lod1  : texture_3d<f32>            (LOD 1 atlas)
//   lookup_subgrid_pool_lod2  : texture_3d<f32>            (LOD 2 atlas)
//   lookup_subgrid_pool_lod3  : texture_3d<f32>            (LOD 3 atlas)
//   lookup_pool_sampler       : sampler                    (Linear + ClampToEdge)
//   lookup_chunk_lod_mask     : storage<read>  ChunkLodMask  (active LODs bitmask)
//   lookup_uniform            : uniform        LookupUniform
//
// This file deliberately does NOT declare `@group/@binding` for those
// globals. The Rust helper `crate::sparse::lookup_wgsl(group, root_b,
// pool_bs[4], uniform_b, sampler_b, mask_b)` prepends the
// `var<...>` declarations with the caller's chosen slots.
//
// # Canonical root_indices binding
//
// Each chunk owns four per-LOD `root_indices` buffers, but post-
// cascade (after the full `chunk_lod → classify → populate →
// downsample` chain has run) they all hold the same value at every
// cell — the downsample stages copy LOD 0's `subgrid_idx` forward
// across LODs. The lookup therefore binds *one* of them, by
// convention `root_indices_buffer(0)`. Calling lookup before the
// cascade has run is undefined.
//
// # LOD selection
//
// `target_voxel_size` is the world-space voxel pitch the consumer
// would like — typically pixel-size at the sample's distance for a
// raymarcher, or `LOD_VOXEL_SIZES[0]` for the Edit Baker (max
// detail). The lookup picks the *coarsest acceptable* LOD: the
// largest LOD index `i ≤ desired_lod` that's set in
// `chunk_lod_mask`. Since bit 0 is always set (cascade invariant),
// this always succeeds.
//
// # HW trilinear via per-LOD atlas
//
// Each LOD's atlas is sampled through `textureSampleLevel` with a
// `Linear + ClampToEdge` sampler — the GPU's TMU does the 8-corner
// trilinear blend in one instruction. The atlas tiles include a
// 1-voxel skirt per face containing the neighbouring root cell's
// corner sample, so subgrid seams reconstruct C0-continuous without
// a cross-tile bind dance.

struct LookupUniform {
    // `xyz` = chunk-local `bounds_min` (post-`ActiveOrigin`). `w`
    // reserved.
    bounds_min: vec4<f32>,
    // `xyz` = chunk-local `bounds_max`. `w` reserved.
    bounds_max: vec4<f32>,
    // `x` = base voxel pitch (cell_size at LOD 0). The
    // `lod_for_voxel_size` helper compares
    // `target_voxel_size / cell_size_base` against the per-LOD
    // factors. `yzw` reserved.
    cell_size_base: vec4<f32>,
}

struct LookupChunkLodMask {
    value: u32,
}

const LOOKUP_LOD_COUNT: u32 = 4u;
const LOOKUP_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu;
const LOOKUP_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;

// `LOOKUP_ROOT_DIM` and `LOOKUP_ATLAS_TILES_{X,Y,Z}` are prepended by
// the host helper `crate::sparse::lookup_wgsl` so consumers do not
// need to know the chunk-local sparse geometry to compile their
// pipeline. The values mirror `crate::sparse::ROOT_DIM`,
// `ATLAS_TILES_X`, `ATLAS_TILES_Y`, `ATLAS_TILES_Z` at the time the
// fragment is built — switching the `large-root-grid` feature
// reshapes the const values without touching this body.

// Per-LOD geometry tables. Mirror `crate::sparse::LOD_LEVELS` on the
// host. Materialised as helper functions so the shader switches over
// `lod_chosen` once per lookup, rather than indexing constexpr arrays
// (WGSL has uneven support for runtime indexing of `const` arrays
// across naga backends).
fn lookup_subgrid_dim(lod: u32) -> u32 {
    switch lod {
        case 0u: { return 16u; }
        case 1u: { return 8u; }
        case 2u: { return 4u; }
        default: { return 2u; }
    }
}

fn lookup_tile_dim(lod: u32) -> u32 {
    return lookup_subgrid_dim(lod) + 1u;
}

fn lookup_atlas_dim(lod: u32) -> vec3<f32> {
    let tile = f32(lookup_tile_dim(lod));
    return vec3<f32>(
        f32(LOOKUP_ATLAS_TILES_X) * tile,
        f32(LOOKUP_ATLAS_TILES_Y) * tile,
        f32(LOOKUP_ATLAS_TILES_Z) * tile,
    );
}

// LOD voxel-size factor: `2^lod`. Materialised as a switch so the
// shader stays portable across naga backends (some early targets
// lacked `pow(2.0, f32(lod))` constant folding).
fn lookup_voxel_size_factor(lod: u32) -> f32 {
    switch lod {
        case 0u: { return 1.0; }
        case 1u: { return 2.0; }
        case 2u: { return 4.0; }
        default: { return 8.0; }
    }
}

// Resolve `target_voxel_size` to the LOD index whose voxel pitch
// best matches it: the largest `i` such that
// `factor[i] × cell_size_base ≤ target`. Returns LOD 0 when the
// caller asks for finer detail than we have, LOD 3 when coarser.
fn lookup_lod_for_voxel_size(target_voxel_size: f32, cell_size_base: f32) -> u32 {
    var best: u32 = 0u;
    var i: u32 = 0u;
    loop {
        if (i >= LOOKUP_LOD_COUNT) { break; }
        let factor = lookup_voxel_size_factor(i);
        if (factor * cell_size_base <= target_voxel_size) {
            best = i;
        }
        i = i + 1u;
    }
    return best;
}

// Finite far-from-surface sentinel. See S6 docs for the rationale —
// the LOD 0 cell pitch sets the magnitude (cells are LOD-independent).
fn sparse_sdf_far_value(bounds_min: vec3<f32>, bounds_max: vec3<f32>) -> f32 {
    let cell_size = (bounds_max - bounds_min) / f32(LOOKUP_ROOT_DIM);
    return max(max(cell_size.x, cell_size.y), cell_size.z) * 2.0;
}

// Per-LOD atlas sample. Switches once on `lod`; the texture binding
// is pinned in each branch since WGSL has no first-class array of
// texture bindings.
fn sparse_sample_lod_atlas(lod: u32, tex_coord: vec3<f32>) -> f32 {
    switch lod {
        case 0u: {
            return textureSampleLevel(
                lookup_subgrid_pool_lod0, lookup_pool_sampler, tex_coord, 0.0,
            ).x;
        }
        case 1u: {
            return textureSampleLevel(
                lookup_subgrid_pool_lod1, lookup_pool_sampler, tex_coord, 0.0,
            ).x;
        }
        case 2u: {
            return textureSampleLevel(
                lookup_subgrid_pool_lod2, lookup_pool_sampler, tex_coord, 0.0,
            ).x;
        }
        default: {
            return textureSampleLevel(
                lookup_subgrid_pool_lod3, lookup_pool_sampler, tex_coord, 0.0,
            ).x;
        }
    }
}

// Pick the coarsest acceptable LOD (highest bit ≤ desired in mask).
// Cascade invariant: bit 0 is always set, so `mask_le_desired` is
// always non-zero and `firstLeadingBit` returns a valid index.
fn sparse_choose_lod(desired_lod: u32, chunk_lod_mask: u32) -> u32 {
    let upper_inclusive = (1u << (desired_lod + 1u)) - 1u;
    let mask_le = chunk_lod_mask & upper_inclusive;
    return firstLeadingBit(mask_le);
}

fn sparse_sdf_lookup(world_pos: vec3<f32>, target_voxel_size: f32) -> f32 {
    let bounds_min = lookup_uniform.bounds_min.xyz;
    let bounds_max = lookup_uniform.bounds_max.xyz;
    let cell_size_base = lookup_uniform.cell_size_base.x;
    let far = sparse_sdf_far_value(bounds_min, bounds_max);

    // Out-of-bounds short-circuit. `>= bounds_max` (not `>`) keeps the
    // upper face exclusive so a sample exactly at `bounds_max` does
    // not index `cell == ROOT_DIM` and walk off the root grid.
    if (any(world_pos < bounds_min) || any(world_pos >= bounds_max)) {
        return far;
    }

    let mask = lookup_chunk_lod_mask.value;
    let desired_lod = lookup_lod_for_voxel_size(target_voxel_size, cell_size_base);
    let lod_chosen = sparse_choose_lod(desired_lod, mask);

    let extent = bounds_max - bounds_min;
    let cell_size = extent / f32(LOOKUP_ROOT_DIM);

    let local_in_root = (world_pos - bounds_min) / cell_size;
    let cell = vec3<u32>(floor(local_in_root));
    let cell_idx = cell.x
        + cell.y * LOOKUP_ROOT_DIM
        + cell.z * LOOKUP_ROOT_DIM * LOOKUP_ROOT_DIM;

    // Canonical root_indices: post-cascade, every per-LOD root_indices
    // buffer holds the same `subgrid_idx` for the same cell, so we
    // bind only one (LOD 0's by host convention) and reuse it across
    // the chosen LOD.
    let subgrid_idx = lookup_root_indices[cell_idx];
    if (subgrid_idx == LOOKUP_EMPTY_ROOT_SENTINEL
        || subgrid_idx == LOOKUP_ALLOC_FAILED_SENTINEL) {
        return far;
    }

    let cell_min = bounds_min + vec3<f32>(cell) * cell_size;
    let subgrid_dim = lookup_subgrid_dim(lod_chosen);
    let subgrid_dim_f = f32(subgrid_dim);
    let local_voxel = (world_pos - cell_min) / cell_size * subgrid_dim_f;
    // Clamp to `[0, subgrid_dim]` (skirt-inclusive). Without this, a
    // sample at the cell's far face could pick up f32 rounding into
    // `> subgrid_dim` and the sampler would read into the next atlas
    // tile — atlas neighbours are not SDF neighbours.
    let local_voxel_clamped = clamp(
        local_voxel,
        vec3<f32>(0.0),
        vec3<f32>(subgrid_dim_f),
    );

    let tile_dim = lookup_tile_dim(lod_chosen);
    // Standard 3D index decode: `subgrid_idx = x + y * X + z * X * Y`.
    // With `LOOKUP_ATLAS_TILES_Y == 1u` (default) this collapses to
    // the historical `(x, 0, z)` layout; with Y > 1 (large-root-grid)
    // the Y axis carries the second slab of tiles.
    let tile_x = subgrid_idx % LOOKUP_ATLAS_TILES_X;
    let tile_y = (subgrid_idx / LOOKUP_ATLAS_TILES_X) % LOOKUP_ATLAS_TILES_Y;
    let tile_z = subgrid_idx / (LOOKUP_ATLAS_TILES_X * LOOKUP_ATLAS_TILES_Y);
    let tile_origin = vec3<f32>(
        f32(tile_x * tile_dim),
        f32(tile_y * tile_dim),
        f32(tile_z * tile_dim),
    );
    let atlas_dim = lookup_atlas_dim(lod_chosen);
    // `+ 0.5` shifts to texel centres so an integer `local_voxel`
    // (e.g. exactly at voxel `(0,0,0)`) reads that texel's stored
    // value with no fractional contribution.
    let tex_coord = (tile_origin + local_voxel_clamped + vec3<f32>(0.5)) / atlas_dim;
    return sparse_sample_lod_atlas(lod_chosen, tex_coord);
}
