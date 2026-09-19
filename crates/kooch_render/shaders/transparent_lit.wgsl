// transparent_lit.wgsl — one transparent fragment, reconstructed exactly as a visibility-buffer
// sample would be and lit by Inti (#452). Linear radiance and coverage, or a coverage of -1 where
// the surface's clip cuts it.

fn transparent_lit(slot: u32, triangle: u32, frag_coord: vec2<f32>) -> vec4<f32> {
    var surf = resolve_surface(slot, triangle, frag_coord);
    // Where both faces are drawn, a back face is lit from the side the viewer sees.
    if (dot(surf.world_normal, inti.camera_position - surf.world_position) < 0.0) {
        surf.world_normal = -surf.world_normal;
    }
    let shaded = surface(surface_input(surf, frag_coord));
    // Below its clip: no coverage, and a negative one tells the raster frames to discard.
    if (shaded.alpha < shaded.alpha_clip) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    var radiance = vec3<f32>(0.0);
    if (!SURFACE_UNLIT) {
        radiance = inti_shade(
            surf.world_position, shaded.normal, shaded.base_color, shaded.metallic,
            shaded.roughness, frag_coord, surf.flags);
    }
    // Display units, as the other frames: 1.0 is full brightness at any exposure.
    radiance += shaded.emissive / max(inti.exposure, 1e-8);
    return vec4<f32>(radiance, clamp(shaded.alpha, 0.0, 1.0));
}
