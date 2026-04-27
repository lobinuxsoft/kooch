// morton.wgsl — compute the 30-bit Morton code of every input AABB's
// centre, normalised to the scene bounds.
//
// One workgroup per 256 items; each thread handles one item. Output
// `morton_codes[i]` parallels `aabbs[i]` for the subsequent radix sort
// (PR-3 subtask 2).
//
// The encoding is byte-identical to `MortonCode::from_normalized` in
// `morton.rs`. CPU and GPU builds emit the same values, validated by
// the consistency test in `gpu/morton.rs::tests`.

struct GpuAabb {
    min_x: f32,
    min_y: f32,
    min_z: f32,
    _pad0: f32,
    max_x: f32,
    max_y: f32,
    max_z: f32,
    _pad1: f32,
}

struct SceneBounds {
    // World-space min of the union of every input AABB.
    min: vec3<f32>,
    _pad0: f32,
    // Per-axis 1.0 / extent. 0.0 on degenerate axes (zero extent) —
    // consumers treat the result as "all items collapse to cell 0".
    inv_extent: vec3<f32>,
    // Number of items. Threads with `gid.x >= count` early-out.
    count: u32,
}

@group(0) @binding(0) var<storage, read> aabbs: array<GpuAabb>;
@group(0) @binding(1) var<uniform> scene: SceneBounds;
@group(0) @binding(2) var<storage, read_write> morton_codes: array<u32>;

// Sean-Anderson bit-twiddling: insert two zero bits between each of
// the input's low 10 bits. Identical to the CPU `expand_bits_10`.
fn expand_bits_10(v: u32) -> u32 {
    var x = v & 0x3FFu;
    x = (x | (x << 16u)) & 0x030000FFu;
    x = (x | (x <<  8u)) & 0x0300F00Fu;
    x = (x | (x <<  4u)) & 0x030C30C3u;
    x = (x | (x <<  2u)) & 0x09249249u;
    return x;
}

@compute @workgroup_size(256)
fn morton_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if i >= scene.count {
        return;
    }

    let a = aabbs[i];
    let centre = vec3<f32>(
        (a.min_x + a.max_x) * 0.5,
        (a.min_y + a.max_y) * 0.5,
        (a.min_z + a.max_z) * 0.5,
    );
    // (centre - scene.min) * scene.inv_extent. Multiplies by zero on
    // degenerate axes so out-of-range inputs land at cell 0 rather than
    // wrapping.
    let normalized =
        (centre - scene.min) * scene.inv_extent;
    let scaled = clamp(
        normalized * 1024.0,
        vec3<f32>(0.0, 0.0, 0.0),
        vec3<f32>(1023.0, 1023.0, 1023.0),
    );
    let xi = u32(scaled.x);
    let yi = u32(scaled.y);
    let zi = u32(scaled.z);
    morton_codes[i] = expand_bits_10(xi) | (expand_bits_10(yi) << 1u) | (expand_bits_10(zi) << 2u);
}
