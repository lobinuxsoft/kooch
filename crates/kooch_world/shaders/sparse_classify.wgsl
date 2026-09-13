// Flags root cells within one cell-diagonal of the SDF surface (Lipschitz cone), gated on bit
// `CLASSIFY_LOD_IDX` of the mask; the output is an indirect-ready compaction, so populate needs no
// readback.

const CLASSIFY_EMPTY_ROOT_SENTINEL: u32 = 0xFFFFFFFFu;
const CLASSIFY_ALLOC_FAILED_SENTINEL: u32 = 0xFFFFFFFEu;

override CLASSIFY_LOD_IDX: u32 = 0u;
// Default values match the no-feature build (`ROOT_DIM = 16`,
// `ROOT_CELLS = 4096`). Pinned per pipeline by `ClassifyPass::new`
// to follow `crate::sparse::ROOT_DIM` and `crate::sparse::ROOT_CELLS`.
override CLASSIFY_ROOT_DIM: u32 = 16u;
override CLASSIFY_ROOT_CELLS: u32 = 4096u;

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

    // Empty and failed cells both re-classify: classify must stay idempotent against allocate, and
    // a failed allocation gets another chance once the pool drains.
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
