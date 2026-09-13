// `sparse_sdf_lookup(world_pos, target_voxel_size)` over globals `lookup_wgsl` declares. Uses LOD
// 0's `root_indices` for every LOD and the coarsest LOD in the mask fine enough; the tile skirt
// keeps trilinear C0.

struct LookupUniform {
    // `xyz` = chunk-local `bounds_min` (post-`ActiveOrigin`). `w`
    // reserved.
    bounds_min: vec4<f32>,
    // `xyz` = chunk-local `bounds_max`. `w` reserved.
    bounds_max: vec4<f32>,
    // `x` = LOD 0 cell size, which `lod_for_voxel_size` scales by each LOD's factor. `yzw`
    // reserved.
    cell_size_base: vec4<f32>,
}

struct LookupChunkLodMask {
    value: u32,
}

const LOOKUP_LOD_COUNT: u32 = 4u;
const LOOKUP_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu;
const LOOKUP_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;

// `LOOKUP_ROOT_DIM` and `LOOKUP_ATLAS_TILES_*` are prepended by `lookup_wgsl`, so the
// `large-root-grid` feature reshapes them without touching this body.

// Functions rather than `const` arrays: runtime indexing of const arrays is unevenly supported
// across naga backends.
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

// The largest LOD with `factor × cell_size_base ≤ target`: LOD 0 when asking for finer than exists,
// LOD 3 when coarser.
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

    // After the cascade every LOD's `root_indices` holds the same index, so LOD 0's serves them
    // all.
    let subgrid_idx = lookup_root_indices[cell_idx];
    if (subgrid_idx == LOOKUP_EMPTY_ROOT_SENTINEL
        || subgrid_idx == LOOKUP_ALLOC_FAILED_SENTINEL) {
        return far;
    }

    let cell_min = bounds_min + vec3<f32>(cell) * cell_size;
    let subgrid_dim = lookup_subgrid_dim(lod_chosen);
    let subgrid_dim_f = f32(subgrid_dim);
    let local_voxel = (world_pos - cell_min) / cell_size * subgrid_dim_f;
    // Clamped to the skirt, or f32 rounding at the far face reads the next atlas tile — which is
    // not the neighbouring cell.
    let local_voxel_clamped = clamp(
        local_voxel,
        vec3<f32>(0.0),
        vec3<f32>(subgrid_dim_f),
    );

    let tile_dim = lookup_tile_dim(lod_chosen);
    // `subgrid_idx = x + y·X + z·X·Y`; `Y` is 1 unless `large-root-grid` adds a second slab.
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
