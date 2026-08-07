// contact_shadow.wgsl — screen-space contact shadows (#735).
//
// Cascaded shadow maps are correct at range and worst at contact: at the
// texel density a cascade can afford, the few centimetres where an
// object meets the ground are exactly where its shadow detaches or
// swims. That gap is what makes a rendered object look like it floats
// over a scene rather than stands in it. A short ray marched through the
// depth buffer fixes that band and nothing else, so the two compose
// rather than compete.
//
// Ported from Bevy 0.19 (`contact_shadows.rs` + `calculate_contact_shadow`
// in `pbr_functions.wgsl`, over their `bevy_pbr::raymarch` module, itself
// a port of Tomasz Stachowiak's `raymarch.hlsl`). Their defaults —
// 16 linear steps, 0.1 m thickness, 0.3 m length — are what this ships.
//
// # Why this chunk declares its own uniforms
//
// It is CONCATENATED into both shading paths, and the two declare their
// camera and screen uniforms at different points in their own text — the
// R32 compute path declares them *after* the shading model, so a chunk
// that referenced them could not sit ahead of it. Naga resolves top to
// bottom. Carrying its own view uniform costs 96 bytes per view and buys
// a chunk that composes anywhere, identically, in both paths.
//
// # The contract
//
// `inti_pbr.wgsl` calls `inti_contact_shadow`; whoever composes the
// shading model has to concatenate this chunk (or the stub) ahead of it.

struct ContactShadowView {
    /// World → clip for this view. The march runs in NDC, so the ray's
    /// two endpoints are projected through this and interpolated there.
    view_proj: mat4x4<f32>,
    /// `linear_z = x / (y - ndc_z)` — the exact inverse of this engine's
    /// reversed-Z projection, in metres from the camera plane.
    ///
    /// 🔴 NOT Bevy's `1.0 / ndc_z`. That identity holds only for a
    /// reversed-Z projection with an *infinite* far plane, and
    /// `perspective_rh_reverse_z` takes a finite one. With
    /// `r = far / (near - far)`, `x = r * near` and `y = 1 + r`; as
    /// `far` grows this collapses back to Bevy's form, which is the
    /// check that it is the same maths and not a different one.
    depth_to_linear: vec2<f32>,
    /// How far along the ray to light the march travels, in world units.
    /// Screen-space cost is independent of this: a longer ray is the same
    /// step count spread wider, which is why the technique costs the same
    /// on a crate and on a moon.
    ray_length: f32,
    /// The depth buffer is 2.5D — it records a surface, not a solid. This
    /// is the thickness every fragment is assumed to have, in world
    /// units. Too small and shadows detach; too large and thin geometry
    /// casts a shadow through everything behind it.
    thickness: f32,
    /// Steps along the ray. **Zero disables the whole feature** — the
    /// early-out below is the "off" switch, so a scene that never enables
    /// contact shadows pays one uniform read per light.
    linear_steps: u32,
    /// Frame counter, for the jitter. Without it the noise pattern is
    /// frozen into the image and reads as a texture rather than as noise.
    frame: u32,
    _pad: vec2<u32>,
}

@group(0) @binding({{CONTACT_SHADOW_UBO_BINDING}}) var<uniform> contact_shadow: ContactShadowView;
// The scene depth buffer, sampled rather than attached — during shading
// the material-depth target is what is bound, so this one is free to be
// read. `textureLoad` and no sampler on purpose: a screen-space march
// wants the exact texel, and the bilinear tap below is reconstructed by
// hand precisely so it can be paired with the point tap.
@group(0) @binding({{CONTACT_SHADOW_DEPTH_BINDING}}) var contact_shadow_depth: texture_depth_2d;

/// Linear distance from the camera plane, in world units.
fn contact_shadow_linear_z(ndc_z: f32) -> f32 {
    return contact_shadow.depth_to_linear.x
        / (contact_shadow.depth_to_linear.y - ndc_z);
}

fn contact_shadow_ndc_to_uv(ndc: vec2<f32>) -> vec2<f32> {
    return ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
}

fn contact_shadow_texel(texel: vec2<i32>, size: vec2<i32>) -> f32 {
    return textureLoad(contact_shadow_depth, clamp(texel, vec2<i32>(0), size - 1), 0);
}

fn contact_shadow_depth_nearest(uv: vec2<f32>, size: vec2<f32>) -> f32 {
    let coord = uv * size - vec2<f32>(0.5);
    return contact_shadow_texel(vec2<i32>(floor(coord + vec2<f32>(0.5))), vec2<i32>(size));
}

fn contact_shadow_depth_bilinear(uv: vec2<f32>, size: vec2<f32>) -> f32 {
    let coord = uv * size - vec2<f32>(0.5);
    let base = vec2<i32>(floor(coord));
    let f = coord - floor(coord);
    let isize = vec2<i32>(size);
    let d00 = contact_shadow_texel(base, isize);
    let d10 = contact_shadow_texel(base + vec2<i32>(1, 0), isize);
    let d01 = contact_shadow_texel(base + vec2<i32>(0, 1), isize);
    let d11 = contact_shadow_texel(base + vec2<i32>(1, 1), isize);
    return mix(mix(d00, d10, f.x), mix(d01, d11, f.x), f.y);
}

/// Interleaved gradient noise (Jimenez '14). Decorrelates the first step
/// of neighbouring pixels so the march's quantization becomes dither
/// rather than banding, and rotates per frame.
fn contact_shadow_noise(frag_coord: vec2<f32>, frame: u32) -> f32 {
    let xy = frag_coord + 5.588238 * f32(frame % 64u);
    return fract(52.9829189 * fract(0.06711056 * xy.x + 0.00583715 * xy.y));
}

struct ContactShadowSample {
    /// Signed: negative once the ray has descended below the surface.
    distance: f32,
    /// How far past the surface the ray is. Compared against `thickness`
    /// to decide whether this counts as an occlusion or as the ray
    /// travelling behind something too thin to have stopped it.
    penetration: f32,
    valid: bool,
}

/// The distance function the root finder searches.
///
/// Reads the depth buffer **twice**, point and bilinear, and only calls
/// occlusion when the ray is below both. Bevy inherited this from
/// Stachowiak and the reasoning is worth keeping: the point tap turns
/// the scene into stacked bricks and produces false occlusion on smooth
/// slopes; the bilinear tap reconstructs the smooth surface but shrink-
/// wraps object boundaries, so it shadows across silhouettes. Each one
/// rejects the other's artefact.
fn contact_shadow_evaluate(ray_ndc: vec3<f32>, size: vec2<f32>) -> ContactShadowSample {
    let uv = contact_shadow_ndc_to_uv(ray_ndc.xy);
    let ray_z = contact_shadow_linear_z(ray_ndc.z);

    let smooth_z = contact_shadow_linear_z(contact_shadow_depth_bilinear(uv, size));
    let sharp_z = contact_shadow_linear_z(contact_shadow_depth_nearest(uv, size));
    let far_z = max(smooth_z, sharp_z);
    let near_z = min(smooth_z, sharp_z);

    // Relative, so it scales with the precision actually available at
    // this distance instead of being a fixed number of metres that is
    // enormous up close and nothing at range.
    let bias = 0.000002;

    var res: ContactShadowSample;
    res.distance = far_z * (1.0 + bias) - ray_z;
    res.penetration = ray_z - near_z;
    // Marching *behind* surfaces on purpose: a ray that dives further
    // than `thickness` under something has passed under a surface too
    // thin to have blocked it, and should carry on looking for a real
    // occluder rather than stopping there.
    res.valid = res.penetration < contact_shadow.thickness;
    return res;
}

/// How much of the light survives the march. `1.0` = unoccluded.
///
/// Returns a **fade**, not a boolean: the hit is graded by how deep into
/// the occluder it landed, so the edge of a contact shadow softens
/// instead of switching. A hard binary here reads as aliasing, which is
/// the artefact the technique exists to remove.
///
/// ⚠️ Screen-space: an occluder outside the frame or behind the camera
/// does not exist. The ray is clipped to the frustum and a ray that
/// leaves it simply reports no hit, which is a shadow that fades out at
/// the screen edge rather than one that pops.
fn inti_contact_shadow(
    world_position: vec3<f32>,
    to_light: vec3<f32>,
    frag_coord: vec2<f32>,
) -> f32 {
    if (contact_shadow.linear_steps == 0u) {
        return 1.0;
    }

    let start_clip = contact_shadow.view_proj * vec4<f32>(world_position, 1.0);
    if (start_clip.w <= 0.0) {
        return 1.0;
    }
    var start_ndc = start_clip.xyz / start_clip.w;

    let end_world = world_position + to_light * contact_shadow.ray_length;
    let end_clip = contact_shadow.view_proj * vec4<f32>(end_world, 1.0);
    let end_ndc = end_clip.xyz / (select(-1.0, 1.0, end_clip.w >= 0.0) * max(1e-10, abs(end_clip.w)));

    // Clip both ends to the view frustum, in NDC. `sign(end_ndc.z)`
    // mirrors a ray whose far end projected from behind the eye, where
    // the perspective divide reverses the direction.
    var delta = (end_ndc - start_ndc) * sign(end_ndc.z);
    let end_point = start_ndc + delta;

    // The edge the ray is coming FROM on each axis, so a start outside
    // the frustum is pulled to where it enters. Reversed-Z, so z runs to
    // 0 at the far plane and the pair is `(−1, −1, 0)` / `(1, 1, 1)`.
    let near_edge = select(vec3<f32>(-1.0, -1.0, 0.0), vec3<f32>(1.0), delta < vec3<f32>(0.0));
    let to_near = (near_edge - start_ndc) / delta;
    start_ndc += delta * max(0.0, max(to_near.x, to_near.y));

    // …and the edge it is heading toward, so the ray stops at the screen
    // rather than sampling outside it.
    delta = end_point - start_ndc;
    let far_edge = select(vec3<f32>(-1.0, -1.0, 0.0), vec3<f32>(1.0), delta >= vec3<f32>(0.0));
    let to_far = (far_edge - start_ndc) / delta;
    delta *= min(1.0, min(min(to_far.x, to_far.y), to_far.z));
    let end = start_ndc + delta;

    let size = vec2<f32>(textureDimensions(contact_shadow_depth));
    let start_uv = contact_shadow_ndc_to_uv(start_ndc.xy);
    let end_uv = contact_shadow_ndc_to_uv(end.xy);

    // A ray a fraction of a pixel long has nothing to sample between its
    // ends; capping the step count at its length in pixels spends the
    // taps only where there is something new to read.
    let ray_px = length((end_uv - start_uv) * size);
    let steps = u32(max(2.0, min(f32(contact_shadow.linear_steps), floor(ray_px))));

    let jitter = contact_shadow_noise(frag_coord, contact_shadow.frame);

    var hit = ContactShadowSample(0.0, 0.0, false);
    var intersected = false;
    for (var step = 0u; step < steps; step += 1u) {
        let t = (f32(step) + jitter) / f32(steps);
        let candidate = contact_shadow_evaluate(mix(start_ndc, end, t), size);
        if (candidate.distance < 0.0 && candidate.valid) {
            hit = candidate;
            intersected = true;
            break;
        }
    }

    if (!intersected
        || hit.penetration >= contact_shadow.thickness
        || hit.distance >= contact_shadow.thickness) {
        return 1.0;
    }

    // Graded by how deep the hit landed inside the assumed thickness, and
    // in that direction on purpose: a shallow hit is a confident one — the
    // ray stopped right at the surface — while a deep one is about to
    // exit the far side of a fragment the depth buffer only guessed the
    // thickness of, and fades back to lit rather than popping off when
    // the next step crosses the limit. Bevy's remap.
    let frac = hit.penetration / contact_shadow.thickness;
    return clamp((frac - 0.5) / 0.5, 0.0, 1.0);
}
