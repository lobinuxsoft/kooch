// contact_shadow_apply.wgsl — the call site (#735).
//
// Concatenated AFTER `bevy_raymarch.wgsl`. `calculate_contact_shadow` is
// Bevy 0.19's, from `pbr_functions.wgsl:298`, with their bindings
// replaced by this engine's uniform and nothing else changed. The rest
// of the file is ours: the probe that lets a debug view ask what the
// march did, and `inti_contact_shadow`, which is the name the shading
// model calls.

/// Everything the march found, so a debug view can ask questions the
/// returned multiplier cannot answer.
///
/// Built from what Bevy's `DepthRayMarchResult` already reports, so the
/// port stays untouched: **`hit_t` is a lerp factor over the ray**, and
/// a hit within the first step lands under `1 / steps` of it.
struct ContactShadowProbe {
    /// The multiplier. `1.0` = unoccluded.
    shadow: f32,
    hit: bool,
    /// Normalised position of the hit along the ray, `0..=1`.
    hit_t: f32,
    /// Steps the march actually took, after its pixel-length cap.
    steps: u32,
    /// Length of the clipped ray in pixels.
    ray_px: f32,
}

/// Bevy 0.19, `pbr_functions.wgsl:298`. Their `#ifdef BLUE_NOISE_TEXTURE`
/// branch is resolved to the `interleaved_gradient_noise` side, which is
/// what a view without a blue-noise texture takes.
fn calculate_contact_shadow(
    world_position: vec3<f32>,
    frag_coord: vec2<f32>,
    light_dir: vec3<f32>,
    contact_shadow_steps: u32,
) -> ContactShadowProbe {
    let noise = interleaved_gradient_noise(frag_coord, contact_shadow.frame);

    let depth_size = vec2<f32>(textureDimensions(depth_prepass_texture));
    var rm = depth_ray_march_new_from_depth(depth_size);
    depth_ray_march_from_cs(&rm, position_world_to_ndc(world_position));
    depth_ray_march_to_ws(&rm, world_position + light_dir * contact_shadow.ray_length);
    rm.linear_steps = contact_shadow_steps;
    rm.depth_thickness_linear_z = contact_shadow.thickness;
    rm.march_behind_surfaces = true;
    rm.jitter = noise;

    let rm_result = depth_ray_march_march(&rm);

    // Everything below is this engine's, and reads only what the march
    // already computed.
    let ray_px = length(
        (ndc_to_uv(rm.ray_end_cs.xy) - ndc_to_uv(rm.ray_start_cs.xy)) * depth_size);
    var probe = ContactShadowProbe(1.0, rm_result.hit, rm_result.hit_t, rm.linear_steps, ray_px);
    if (rm_result.hit) {
        probe.shadow = clamp((rm_result.hit_penetration_frac - 0.5) / (1.0 - 0.5), 0.0, 1.0);
    }
    return probe;
}

/// How much of the light survives the march. `1.0` = unoccluded.
///
/// ⚠️ Screen-space: an occluder outside the frame or behind the camera
/// does not exist. The ray is clipped to the frustum and a ray that
/// leaves it reports no hit, so a contact shadow fades out at the screen
/// edge rather than popping.
fn inti_contact_shadow(
    world_position: vec3<f32>,
    to_light: vec3<f32>,
    frag_coord: vec2<f32>,
) -> f32 {
    return inti_contact_shadow_probe(world_position, to_light, frag_coord).shadow;
}

fn inti_contact_shadow_probe(
    world_position: vec3<f32>,
    to_light: vec3<f32>,
    frag_coord: vec2<f32>,
) -> ContactShadowProbe {
    if (contact_shadow.linear_steps == 0u) {
        return ContactShadowProbe(1.0, false, 0.0, 0u, 0.0);
    }
    return calculate_contact_shadow(
        world_position, frag_coord, to_light, contact_shadow.linear_steps);
}

/// What the march saw at this point, as colour.
///
/// The three things that look identical in a shaded frame and have
/// different fixes:
///
/// - **red** — a hit inside the **first step**. That sample sits a
///   fraction of the ray from where the ray started, so it is as likely
///   to be the surface occluding itself as a real occluder. Speckle in
///   this colour is the surface fighting its own depth, not geometry.
/// - **green** — a hit further along, brighter the later it is. This is
///   what a real contact shadow looks like.
/// - **blue** — the ray was under two pixels long, so the march had
///   nowhere to go and every sample landed on the origin. Separates
///   "the maths is wrong" from "there was nothing to march".
/// - **grey** — marched and found nothing, which is most of a frame.
fn inti_contact_shadow_debug(probe: ContactShadowProbe) -> vec3<f32> {
    if (probe.ray_px < 2.0) {
        return vec3<f32>(0.15, 0.3, 1.0);
    }
    if (!probe.hit) {
        return vec3<f32>(0.12);
    }
    let first_step = 1.0 / max(1.0, f32(probe.steps));
    if (probe.hit_t <= first_step) {
        return vec3<f32>(1.0, 0.1, 0.1);
    }
    // Later is more credible, so brighter.
    return vec3<f32>(0.1, 0.35 + 0.65 * saturate(probe.hit_t), 0.1);
}
