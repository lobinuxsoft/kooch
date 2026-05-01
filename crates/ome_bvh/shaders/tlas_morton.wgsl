// tlas_morton.wgsl — compute the 30-bit Morton code of every live
// chunk's centre, normalised to the scene-wide bounds.
//
// Mirror of `morton.wgsl` (BLAS): same encoding, same workgroup size.
// Difference: the input is `array<ChunkDescriptor>` (where the centre
// is `(aabb_min + aabb_max) / 2`), not `array<GpuAabb>` directly. The
// distinction is what makes this a TLAS pass — every chunk reduces to
// a single Morton code keyed off its world-space centre.
//
// Encoding byte-identity with the CPU `MortonCode::from_normalized` is
// pinned by the integration test in `tests/tlas_morton.rs`. The
// `expand_bits_10` helper is **kept in sync with `morton.wgsl`** —
// change both if the encoding ever changes (no WGSL include support
// for that level of code reuse on stable wgpu yet).

struct ChunkDescriptor {
    aabb_min: vec3<f32>,
    first_node: u32,
    aabb_max: vec3<f32>,
    node_count: u32,
    first_leaf: u32,
    leaf_count: u32,
    first_primitive: u32,
    primitive_count: u32,
    max_smoothness_radius: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

struct SceneBounds {
    // World-space min of the union of every live chunk's AABB.
    min: vec3<f32>,
    _pad0: f32,
    // Per-axis 1.0 / extent. 0.0 on degenerate axes (zero extent) —
    // consumers treat the result as "all chunks collapse to cell 0".
    inv_extent: vec3<f32>,
    // Number of live chunks. Threads with `gid.x >= count` early-out.
    count: u32,
}

struct TlasConfig {
    // Live chunk count (TLAS leaf count). Equals `scene.count`; carried
    // separately so subsequent Karras passes (leaves, internal, aabb)
    // share a single uniform layout.
    n: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<storage, read> chunk_descriptors: array<ChunkDescriptor>;
@group(0) @binding(1) var<uniform> scene: SceneBounds;
@group(0) @binding(2) var<storage, read_write> mortons: array<u32>;
@group(0) @binding(3) var<uniform> cfg: TlasConfig;

// Sean-Anderson bit-twiddling: insert two zero bits between each of
// the input's low 10 bits. Identical to the CPU `expand_bits_10` and
// to the BLAS `morton.wgsl` helper.
fn expand_bits_10(v: u32) -> u32 {
    var x = v & 0x3FFu;
    x = (x | (x << 16u)) & 0x030000FFu;
    x = (x | (x <<  8u)) & 0x0300F00Fu;
    x = (x | (x <<  4u)) & 0x030C30C3u;
    x = (x | (x <<  2u)) & 0x09249249u;
    return x;
}

@compute @workgroup_size(256)
fn tlas_morton_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= cfg.n {
        return;
    }

    let d = chunk_descriptors[i];
    let centre = (d.aabb_min + d.aabb_max) * 0.5;

    let normalized = (centre - scene.min) * scene.inv_extent;
    let scaled = clamp(
        normalized * 1024.0,
        vec3<f32>(0.0, 0.0, 0.0),
        vec3<f32>(1023.0, 1023.0, 1023.0),
    );
    let xi = u32(scaled.x);
    let yi = u32(scaled.y);
    let zi = u32(scaled.z);
    mortons[i] = expand_bits_10(xi) | (expand_bits_10(yi) << 1u) | (expand_bits_10(zi) << 2u);
}
