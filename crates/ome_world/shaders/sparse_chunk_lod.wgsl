// Sparse chunk LOD pass — pick the active LOD set for each chunk
// based on its distance to a global "active origin" (player position
// in the planet-scale design). 1 dispatch sets `chunk_lod_mask` to
// a `u32` bitmask: bit `i` = "LOD `i` is active for this chunk".
//
// # Distance thresholds
//
// Default thresholds — written by the host into `lod_distance_thresholds`:
//
// ```
// LOD 0  ∈  [0,    100m)
// LOD 1  ∈  [100,  500m)
// LOD 2  ∈  [500,  2km)
// LOD 3  ∈  [2km,  +∞)
// ```
//
// Bit 0 is *always* set regardless of distance — every cascade
// dispatch downsample reads from LOD 0 as the source, so LOD 0 must
// be populated for every chunk. Higher LOD bits set when the chunk
// centre falls inside that LOD's distance window.
//
// # N = 1 today, scales to N chunks
//
// Today the grid runs one chunk at a time, so the dispatch is
// `(1, 1, 1)` and `chunk_lod_mask` is a single `u32`. The shader is
// already structured for `array<u32>` indexing (it just reads
// invocation 0); when #313 lands the host upgrades the buffer to
// `array<u32>` and dispatches `(N, 1, 1)` over the chunk array — no
// shader change needed.

struct ChunkLodUniform {
    // `xyz` = active origin in world space (player / camera position).
    // `w` reserved.
    active_origin: vec4<f32>,
    // `xyz` = LOD distance thresholds in metres. `x` = LOD 0→1
    // boundary, `y` = LOD 1→2, `z` = LOD 2→3. Beyond `.z`, only LOD 3
    // is active (plus the always-on bit 0).
    lod_distance_thresholds: vec4<f32>,
    // Chunk centre — for now a single chunk per dispatch. `w` is the
    // chunk's bounding sphere radius (currently unused; reserved for
    // distance-to-AABB upgrades that respect chunk size).
    chunk_center_radius: vec4<f32>,
}

@group(0) @binding(0) var<uniform> chunk_lod_uniform: ChunkLodUniform;
@group(0) @binding(1) var<storage, read_write> chunk_lod_mask: u32;

@compute @workgroup_size(1)
fn chunk_lod_main() {
    let origin = chunk_lod_uniform.active_origin.xyz;
    let centre = chunk_lod_uniform.chunk_center_radius.xyz;
    let thresholds = chunk_lod_uniform.lod_distance_thresholds.xyz;
    let dist = distance(origin, centre);

    // Bit 0 is always on — the cascade's downsample chain reads from
    // LOD 0 as the source, so LOD 0 must be populated even for chunks
    // far from the active origin.
    var mask: u32 = 1u;
    if (dist < thresholds.x) {
        mask = mask | 0x1u;
    } else if (dist < thresholds.y) {
        mask = mask | 0x2u;
    } else if (dist < thresholds.z) {
        mask = mask | 0x4u;
    } else {
        mask = mask | 0x8u;
    }

    chunk_lod_mask = mask;
}
