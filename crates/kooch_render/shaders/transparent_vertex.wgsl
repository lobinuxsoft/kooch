// transparent_vertex.wgsl — the vertex stage every transparent pass rasterises with (#452).
// `visible_meshlets` is the forward list here, not the cull's: one packed (instance, meshlet) per
// draw instance.

struct ForwardOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) slot: u32,
    @location(1) @interpolate(flat) triangle: u32,
}

// 🔴 Tested in the shader, not left to the depth attachment: a fragment that writes storage runs
// before the late depth test rejects it, so one behind the opaque scene would still land.
// Reversed-Z: covered where it is not greater than what the raster kept.
fn opaque_covers(position: vec4<f32>) -> bool {
    return position.z <= textureLoad(depth_prepass_texture, vec2<i32>(position.xy), 0);
}

@vertex
fn vs_forward(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) slot: u32,
) -> ForwardOut {
    let packed = visible_meshlets[slot];
    let inst = instances[packed >> 16u];
    let desc = descriptors[packed & 0xffffu];
    let triangle = vertex_index / 3u;

    var out: ForwardOut;
    out.slot = slot;
    out.triangle = triangle;
    // Past the meshlet's own triangles: a degenerate position the rasteriser drops.
    if (triangle >= desc.triangle_count) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let vertex = global_vertex_id(desc, triangle, vertex_index % 3u);
    out.position = camera.view_proj * corner_world_position(inst, vertex);
    return out;
}
