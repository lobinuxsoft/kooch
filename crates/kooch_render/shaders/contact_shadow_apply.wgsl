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

/// Half the world-space size of one depth texel at `distance`.
///
/// 🔴 The number the march has to clear before its first sample means
/// anything. Bevy never needs it: their march is compiled only behind
/// `#ifdef DEPTH_PREPASS` (`pbr_functions.wgsl:299`), so the ray's
/// origin and the depth buffer came out of the same rasteriser with the
/// same matrix and agree to the bit inside the origin's own texel. This
/// engine reconstructs the origin from the **visibility buffer** by
/// barycentrics instead — a different arithmetic path to the same point
/// — and inside that texel the comparison is decided by the last bit.
/// Which way it falls depends on the jitter, so it lands as salt and
/// pepper rather than as a shape.
///
/// A texel's world size grows linearly with distance, which is why this
/// is computed rather than authored: a fixed offset is a crater up close
/// and nothing at range.
/// How far off the surface the ray has to start for its first sample to
/// mean anything, in world units.
///
/// Two factors, both derived:
///
/// **One texel of depth**, because a sample that has not left the
/// origin's own texel is comparing the fragment against itself.
///
/// **Divided by `n·v`**, because that texel is a *screen* quantity: seen
/// edge-on, one texel spans far more surface than seen face-on, and the
/// depth error inside it grows in exactly that proportion. This is the
/// slope-scaled depth bias every shadow map uses, for the same reason —
/// `inti_sample_cascade` already scales its normal offset by the light's
/// obliquity, and this is the camera's.
///
/// Clamped, because `n·v → 0` at a silhouette and an unbounded lift
/// would launch the ray off the object and lose the contact it exists
/// to find. Four texels is where the surface is steep enough that a
/// contact shadow is a pixel wide anyway.
fn contact_shadow_lift(view_distance: f32, normal: vec3<f32>, to_camera: vec3<f32>) -> f32 {
    let n_dot_v = max(0.25, abs(dot(normal, to_camera)));
    return contact_shadow_texel_world_size(view_distance) / n_dot_v;
}

fn contact_shadow_texel_world_size(view_distance: f32) -> f32 {
    // `view_proj[1][1]` carries `1 / tan(fov_y / 2)` and nothing else on
    // the Y axis, so the frustum's vertical extent at `view_distance` is
    // `2 * view_distance / m11` and one texel is that over the height.
    let m11 = max(1e-6, abs(contact_shadow.view_proj[1][1]));
    let size = vec2<f32>(textureDimensions(depth_prepass_texture));
    return view_distance / (m11 * max(1.0, size.y));
}

/// One march, with the jitter it is handed.
///
/// The body is Bevy 0.19's `calculate_contact_shadow`
/// (`pbr_functions.wgsl:298`) — their setup, their call, their remap.
/// Their `#ifdef BLUE_NOISE_TEXTURE` branch is resolved to the
/// `interleaved_gradient_noise` side, which is what a view without a
/// blue-noise texture takes. What is not theirs: `normal`, for the
/// texel lift, and `jitter` as a parameter rather than a local, so the
/// pair below can vary it.
fn contact_shadow_march(
    world_position_in: vec3<f32>,
    normal: vec3<f32>,
    to_camera: vec3<f32>,
    light_dir: vec3<f32>,
    depth_size: vec2<f32>,
    jitter: f32,
    contact_shadow_steps: u32,
) -> ContactShadowProbe {
    // Lift the ray off the surface before marching. Along the normal
    // rather than along the ray: the gap that matters is to the
    // *surface*, and a ray grazing it would need an unbounded push to
    // clear the same gap.
    //
    // The distance comes straight back out of `ndc.z`, which is the
    // whole point of the projection having no far plane: `near / ndc.z`
    // is metres, with no second uniform and no second code path.
    let origin_ndc = position_world_to_ndc(world_position_in);
    let view_distance = perspective_camera_near() / max(1e-6, origin_ndc.z);
    let world_position =
        world_position_in + normal * contact_shadow_lift(view_distance, normal, to_camera);

    var rm = depth_ray_march_new_from_depth(depth_size);
    depth_ray_march_from_cs(&rm, position_world_to_ndc(world_position));
    depth_ray_march_to_ws(&rm, world_position + light_dir * contact_shadow.ray_length);
    rm.linear_steps = contact_shadow_steps;
    rm.depth_thickness_linear_z = contact_shadow.thickness;
    rm.march_behind_surfaces = true;
    rm.jitter = jitter;

    let rm_result = depth_ray_march_march(&rm);

    let ray_px = length(
        (ndc_to_uv(rm.ray_end_cs.xy) - ndc_to_uv(rm.ray_start_cs.xy)) * depth_size);
    var probe = ContactShadowProbe(1.0, rm_result.hit, rm_result.hit_t, rm.linear_steps, ray_px);
    if (rm_result.hit) {
        probe.shadow = clamp((rm_result.hit_penetration_frac - 0.5) / (1.0 - 0.5), 0.0, 1.0);
    }
    return probe;
}

/// Bevy 0.19's `calculate_contact_shadow` (`pbr_functions.wgsl:298`):
/// **one march, one jitter**.
///
/// An earlier version averaged two marches with complementary jitter to
/// halve the variance. It worked and it is gone, because Bevy casts one
/// ray and Bevy is the reference: the noise it was hiding turned out to
/// be self-occlusion on oblique surfaces, which `contact_shadow_lift`
/// fixes for free. Doubling the rays to cover for a bias that was too
/// small is paying twice for the wrong thing.
fn calculate_contact_shadow(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_camera: vec3<f32>,
    frag_coord: vec2<f32>,
    light_dir: vec3<f32>,
    contact_shadow_steps: u32,
) -> ContactShadowProbe {
    let noise = interleaved_gradient_noise(frag_coord, contact_shadow.frame);
    let depth_size = vec2<f32>(textureDimensions(depth_prepass_texture));
    return contact_shadow_march(
        world_position, normal, to_camera, light_dir, depth_size, noise, contact_shadow_steps);
}

/// How much of the light survives the march. `1.0` = unoccluded.
///
/// ⚠️ Screen-space: an occluder outside the frame or behind the camera
/// does not exist. The ray is clipped to the frustum and a ray that
/// leaves it reports no hit, so a contact shadow fades out at the screen
/// edge rather than popping.
fn inti_contact_shadow(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_camera: vec3<f32>,
    to_light: vec3<f32>,
    frag_coord: vec2<f32>,
) -> f32 {
    return inti_contact_shadow_probe(
        world_position, normal, to_camera, to_light, frag_coord).shadow;
}

/// `to_camera` is passed rather than derived: the lift needs `n·v` and
/// the normal is in **world** space, so the view vector has to be too.
/// This chunk carries `view_proj` and no view matrix, and the shading
/// model already holds the vector — asking for it beats reconstructing
/// it in the wrong space, which is what the first attempt did.
fn inti_contact_shadow_probe(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_camera: vec3<f32>,
    to_light: vec3<f32>,
    frag_coord: vec2<f32>,
) -> ContactShadowProbe {
    if (contact_shadow.linear_steps == 0u) {
        return ContactShadowProbe(1.0, false, 0.0, 0u, 0.0);
    }
    return calculate_contact_shadow(
        world_position, normal, to_camera, frag_coord, to_light, contact_shadow.linear_steps);
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
