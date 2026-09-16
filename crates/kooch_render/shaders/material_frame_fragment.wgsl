// material_frame_fragment.wgsl — the two-pass fragment frame (#440) around a surface shader (#1157),
// lit by Inti (#441).

struct FsInput {
    // @invariant: the Equal depth test against the material-depth target
    // demands bit-identical depth from every per-material draw, so the
    // clip position must not be recomputed differently across draws.
    @builtin(position) @invariant position: vec4<f32>,
}

// Fullscreen triangle cover. Emits this pass's material id as clip-space depth (`screen.material_id
// / 65535`) so the fixed-function `Equal` depth test against the material-depth target admits only
// this material's pixels — the per-material cull, in hardware, with early-Z.
@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FsInput {
    var out: FsInput;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    let z = f32(screen.material_id) / 65535.0;
    out.position = vec4<f32>(x, y, z, 1.0);
    return out;
}

@fragment
fn fs_material(in: FsInput) -> @location(0) vec4<f32> {
    let surf = resolve_vertex_output(in.position);
    let shaded = surface(surface_input(surf, in.position.xy));

    // The debug views (#743). In a production pipeline `inti_debug_is_view` is the stub's literal
    // `false`, so this branch and every view behind it are gone before register allocation.
    if (inti_debug_is_view(screen.debug_mode)) {
        return vec4<f32>(
            inti_debug_view(screen.debug_mode, surf.world_position, shaded.normal, in.position.xy),
            1.0);
    }

    var radiance = inti_shade(
        surf.world_position, shaded.normal, shaded.base_color, shaded.metallic, shaded.roughness,
        in.position.xy, surf.flags);
    // Emissive is radiance the surface produces rather than reflects, so
    // it joins before tonemapping and ignores every light in the scene.
    // 🔴 In display units: divided by the exposure the tonemap is about to apply, so 1.0 is the colour
    // at full brightness whatever the camera's EV. Added raw it was 1.0 × ~0.0009 — invisible.
    radiance += shaded.emissive / max(inti.exposure, 1e-8);

    return vec4<f32>(inti_tonemap(radiance), 1.0);
}
