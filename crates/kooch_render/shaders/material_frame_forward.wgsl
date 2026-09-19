// material_frame_forward.wgsl — transparent surfaces over the opaque scene (#452). The meshlets of
// sorted instances are rasterised, each fragment is reconstructed exactly as a visibility-buffer
// sample would be, lit by Inti, and blended into the linear radiance before the temporal resolve.
//
// `visible_meshlets` here is the forward list, not the cull's: one packed (instance, meshlet) per
// draw instance, far to near.

struct ForwardOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) slot: u32,
    @location(1) @interpolate(flat) triangle: u32,
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

@fragment
fn fs_forward(in: ForwardOut) -> @location(0) vec4<f32> {
    let surf = resolve_surface(in.slot, in.triangle, in.position.xy);
    let shaded = surface(surface_input(surf, in.position.xy));

    var radiance = vec3<f32>(0.0);
    if (!SURFACE_UNLIT) {
        radiance = inti_shade(
            surf.world_position, shaded.normal, shaded.base_color, shaded.metallic,
            shaded.roughness, in.position.xy, surf.flags);
    }
    // Display units, as the other frames: 1.0 is full brightness at any exposure.
    radiance += shaded.emissive / max(inti.exposure, 1e-8);
    // Linear radiance, like the opaque shading it lands on; the tonemap comes after.
    return vec4<f32>(radiance, clamp(shaded.alpha, 0.0, 1.0));
}
