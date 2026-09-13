// contact_shadow_apply.wgsl — the call site (#735).

/// Everything the march found, so a debug view can ask questions the returned multiplier cannot
/// answer.
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
fn contact_shadow_march(
    world_position_in: vec3<f32>,
    normal: vec3<f32>,
    to_camera: vec3<f32>,
    light_dir: vec3<f32>,
    depth_size: vec2<f32>,
    jitter: f32,
    contact_shadow_steps: u32,
) -> ContactShadowProbe {
    // Lift the ray off the surface before marching. Along the normal rather than along the ray: the
    // gap that matters is to the *surface*, and a ray grazing it would need an unbounded push to
    // clear the same gap.
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
        // Bevy's remap, kept.
        probe.shadow = clamp((rm_result.hit_penetration_frac - 0.5) / (1.0 - 0.5), 0.0, 1.0);
    }
    return probe;
}

/// Bevy 0.19's `calculate_contact_shadow` (`pbr_functions.wgsl:298`): **one march, one jitter**.
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

/// Whether the shading model should march once for the strongest light instead of once per light
/// (#845).
fn inti_contact_dominant_only() -> bool {
    return contact_shadow.dominant_only != 0u;
}

/// Punctual lights per pixel that may march. 0 = no cap.
fn inti_contact_max_lights() -> u32 {
    return contact_shadow.max_lights;
}

/// How much of the light survives the march. `1.0` = unoccluded.
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

/// `to_camera` is passed rather than derived: the lift needs `n·v` and the normal is in **world**
/// space, so the view vector has to be too.
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
