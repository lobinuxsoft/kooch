// Sparse downsample cascade — fills LOD `dst` from LOD `src` via a
// 2³ box filter. One pipeline per cascade pair `(0→1, 1→2, 2→3)`,
// instantiated by the host with the destination LOD's geometry pinned
// in the override constants below.
//
// # Dispatch shape
//
// Reuses the source LOD's `populate_indirect_args` buffer
// (`[needs_count_src, 1, 1]`), so this shader sees one workgroup per
// cell that classify marked at the source LOD. Each workgroup
// cooperates on the destination tile's `tile_voxels` voxels via a
// stride loop — 64 threads / workgroup. At LOD 1 (729 voxels) every
// thread runs ≈ 12 iterations; at LOD 3 (27 voxels) most threads sit
// idle, but over-provisioning saves a per-LOD dispatch-shape branch.
//
// # Box filter + skirt invariant
//
// For each destination voxel `(dvx, dvy, dvz)`, the shader reads the
// `2³` source voxels at `(2 dvx + i, 2 dvy + j, 2 dvz + k)` for
// `i, j, k ∈ {0, 1}` and writes their average. The destination
// skirt voxel at `dvx == DST_SUBGRID_DIM` lands at source coords
// `2 × DST_SUBGRID_DIM == SRC_SUBGRID_DIM` (the source skirt). Clamping
// the source coordinate to `[0, SRC_TILE_DIM - 1]` keeps the read
// inside the source tile and degenerates the 2³ filter to a single
// source-skirt voxel — analytically correct because both LOD's skirts
// encode the same `cell_min + cell_size` corner sample.
//
// # Canonical root_indices
//
// The destination's `root_indices[cell_idx]` is set to the source's
// `subgrid_idx` (a copy, not a fresh free-list pop). This preserves
// the invariant that every LOD's root_indices buffer holds the same
// `subgrid_idx` for the same cell post-cascade — the lookup helper
// can then bind any one of them and the others are mirrors.
//
// # Override constants
//
// ```
// LOD 0→1: DST=(8, 9, 729, 32),  SRC=(17, 32)
// LOD 1→2: DST=(4, 5, 125, 32),  SRC=(9,  32)
// LOD 2→3: DST=(2, 3,  27, 32),  SRC=(5,  32)
// ```

const DOWNSAMPLE_WORKGROUP_SIZE: u32 = 64u;
const DOWNSAMPLE_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu;
const DOWNSAMPLE_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;

override DOWNSAMPLE_DST_SUBGRID_DIM: u32 = 8u;
override DOWNSAMPLE_DST_TILE_DIM: u32 = 9u;
override DOWNSAMPLE_DST_TILE_VOXELS: u32 = 729u;
override DOWNSAMPLE_DST_ATLAS_TILES_X: u32 = 32u;
override DOWNSAMPLE_SRC_TILE_DIM: u32 = 17u;
override DOWNSAMPLE_SRC_ATLAS_TILES_X: u32 = 32u;

struct DownsampleNeedsCount {
    value: u32,
}

@group(0) @binding(0) var<storage, read> downsample_src_root_indices: array<u32>;
@group(0) @binding(1) var<storage, read_write> downsample_dst_root_indices: array<u32>;
@group(0) @binding(2) var downsample_src_pool: texture_3d<f32>;
@group(0) @binding(3) var downsample_dst_pool: texture_storage_3d<r16float, write>;
@group(0) @binding(4) var<storage, read> downsample_needs_indices: array<u32>;
@group(0) @binding(5) var<storage, read> downsample_needs_count: DownsampleNeedsCount;

@compute @workgroup_size(64)
fn downsample_main(
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    if (wid.x >= downsample_needs_count.value) {
        return;
    }
    let cell_idx = downsample_needs_indices[wid.x];
    let subgrid_idx = downsample_src_root_indices[cell_idx];
    // Skip empty / alloc-failed cells. Both sentinels are above
    // `MAX_SUBGRIDS` for any reasonable max — a single `>=` test
    // covers both.
    if (subgrid_idx >= DOWNSAMPLE_ALLOC_FAILED_SENTINEL) {
        return;
    }

    let src_tile_x = subgrid_idx % DOWNSAMPLE_SRC_ATLAS_TILES_X;
    let src_tile_z = subgrid_idx / DOWNSAMPLE_SRC_ATLAS_TILES_X;
    let src_tile_origin = vec3<i32>(
        i32(src_tile_x * DOWNSAMPLE_SRC_TILE_DIM),
        0,
        i32(src_tile_z * DOWNSAMPLE_SRC_TILE_DIM),
    );
    let dst_tile_x = subgrid_idx % DOWNSAMPLE_DST_ATLAS_TILES_X;
    let dst_tile_z = subgrid_idx / DOWNSAMPLE_DST_ATLAS_TILES_X;
    let dst_tile_origin = vec3<i32>(
        i32(dst_tile_x * DOWNSAMPLE_DST_TILE_DIM),
        0,
        i32(dst_tile_z * DOWNSAMPLE_DST_TILE_DIM),
    );

    let src_max = i32(DOWNSAMPLE_SRC_TILE_DIM) - 1;

    var i: u32 = lid.x;
    loop {
        if (i >= DOWNSAMPLE_DST_TILE_VOXELS) {
            break;
        }
        let dvz = i / (DOWNSAMPLE_DST_TILE_DIM * DOWNSAMPLE_DST_TILE_DIM);
        let dvy = (i / DOWNSAMPLE_DST_TILE_DIM) % DOWNSAMPLE_DST_TILE_DIM;
        let dvx = i % DOWNSAMPLE_DST_TILE_DIM;

        // 2³ box filter — read 8 source voxels at
        // `(2 dvx + ji, 2 dvy + jj, 2 dvz + jk)` and average. Clamping
        // to the source tile's skirt-inclusive bound keeps reads in
        // the current atlas tile (atlas neighbours are not SDF
        // neighbours).
        var sum: f32 = 0.0;
        for (var jk: u32 = 0u; jk < 2u; jk = jk + 1u) {
            for (var jj: u32 = 0u; jj < 2u; jj = jj + 1u) {
                for (var ji: u32 = 0u; ji < 2u; ji = ji + 1u) {
                    let unclamped = vec3<i32>(
                        i32(dvx * 2u + ji),
                        i32(dvy * 2u + jj),
                        i32(dvz * 2u + jk),
                    );
                    let clamped = clamp(
                        unclamped,
                        vec3<i32>(0),
                        vec3<i32>(src_max),
                    );
                    let texel = src_tile_origin + clamped;
                    sum = sum + textureLoad(downsample_src_pool, texel, 0).x;
                }
            }
        }
        let avg = sum * 0.125;
        let dst_texel = dst_tile_origin + vec3<i32>(i32(dvx), i32(dvy), i32(dvz));
        textureStore(
            downsample_dst_pool,
            dst_texel,
            vec4<f32>(avg, 0.0, 0.0, 0.0),
        );
        i = i + DOWNSAMPLE_WORKGROUP_SIZE;
    }

    // Make tile writes visible before publishing the canonical idx.
    workgroupBarrier();
    if (lid.x == 0u) {
        downsample_dst_root_indices[cell_idx] = subgrid_idx;
    }
}
