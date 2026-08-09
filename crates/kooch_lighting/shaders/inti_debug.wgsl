// inti_debug.wgsl — the debug views, kept OUT of the production shader.
//
// CONCATENATED after `inti_pbr.wgsl`, and only when a debug mode is
// active. Everything here reads bindings and helpers that file already
// declared (`inti`, `inti_lights`, `inti_sample_light`,
// `inti_pick_cascade`, `inti_sample_cascade`, `inti_shadow_coords`), so
// it has no bindings of its own and needs no group substitution.
//
// # Why it is a separate file rather than three `if`s in the shading
//   shader
//
// A branch nothing takes is still code the shader carries. Register
// allocation is worst-case over the whole entry point, so a cascade
// march and a screen-space raymarch sitting in an untaken branch still
// raise the VGPR count, and VGPR count is what caps how many waves an
// SM keeps in flight. Fewer waves is less latency hiding, and latency
// hiding is the entire performance story of an integrated GPU on a
// 10 W budget — the target this engine is held to.
//
// So the game's pipeline concatenates none of this, and cannot pay for
// it. The editor compiles a second pipeline, lazily, the first time
// somebody opens a debug view. `kooch_lighting::inti_debug_shader`
// hands out this text; `INTI_DEBUG_STUB` is what the production build
// gets in its place, and it exists so both pipelines compile against
// the same call sites.

// Discriminants of `MeshletDebugMode`, pinned to the Rust enum by a
// test in `kooch_render`'s `debug.rs`. One copy, here, rather than one
// per shading path: two copies is how a mode ends up meaning different
// things on the R64 and R32 routes, which is a bug no compiler catches.
const INTI_DEBUG_NORMALS: u32 = 11u;
const INTI_DEBUG_SHADOW_CASCADES: u32 = 12u;
const INTI_DEBUG_CONTACT_SHADOWS: u32 = 13u;
// Lowest discriminant handled here. Modes below it are resolved by the
// shading path itself before the surface is even reconstructed.
const INTI_DEBUG_FIRST: u32 = INTI_DEBUG_NORMALS;

// Bevy's cascade colours, and their derivation: hue swept around the
// wheel by cascade index (`shadows.wgsl:265`). Ported rather than
// picked so a capture from this engine and one from Bevy read the same.
// `FRAC_PI_3` and `PI_2` are theirs too, from `bevy_render::maths`.
// Bevy divides the hue by `MAX_CASCADES_PER_LIGHT + 1` so the last
// cascade does not wrap onto the first one's colour.
const FRAME_CASCADE_COUNT_PLUS_ONE: u32 = 5u;
const INTI_FRAC_PI_3: f32 = 1.04719755;
const INTI_PI_2: f32 = 6.28318531;

// `bevy_render::color_operations::hsv_to_rgb`, transcribed.
// H ∈ [0, 2π), S ∈ [0, 1], V ∈ [0, 1].
fn inti_hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    let n = vec3<f32>(5.0, 3.0, 1.0);
    let k = (n + hsv.x / INTI_FRAC_PI_3) % 6.0;
    return hsv.z - hsv.z * hsv.y * max(vec3<f32>(0.0), min(k, min(4.0 - k, vec3<f32>(1.0))));
}

/// What the shadow system sees at this point, as colour.
///
/// # Bevy's colour, and one thing on top
///
/// The hue is `cascade_debug_visualization`'s, computed the same way, so
/// "which cascade covers this" reads identically to a Bevy capture.
///
/// What Bevy does not answer, and #476 needed twice, is **whether the
/// map has an occluder over this point**: "the cascade does not reach
/// here", "the occluder was culled out of the map" and "the sampling is
/// wrong" are three different bugs that look like one missing shadow.
/// So the hue is dimmed where this point is shadowed.
///
/// 🔴 Dimmed by `inti_sample_cascade` — **the same call the shading pass
/// makes**, bias, filter and all. The previous version sampled the atlas
/// raw and deliberately without bias, to show the acne the shading
/// hides; what it actually showed was a screenful of moiré with the
/// cascade boundaries drowned underneath. A debug view whose own noise
/// hides its answer is not a debug view.
///
/// - magenta — no atlas: nothing casts
/// - black — inside no cascade volume, so nothing can be in shadow
/// - dark grey — past the last cascade
/// - cascade hue, bright — lit
/// - cascade hue, dim — shadowed, as the shading pass sees it
fn inti_shadow_debug(world_position: vec3<f32>, n: vec3<f32>, view_depth: f32) -> vec3<f32> {
    if (inti.shadows_enabled == 0u) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    let picked = inti_pick_cascade(view_depth);
    let index = u32(picked.x);
    if (index >= 4u) {
        return vec3<f32>(0.15);
    }
    let cascade = inti.cascades[index];
    if (inti_shadow_coords(cascade, world_position).w == 0.0) {
        return vec3<f32>(0.0);
    }

    let hue = f32(index) / f32(FRAME_CASCADE_COUNT_PLUS_ONE) * INTI_PI_2;
    let colour = inti_hsv_to_rgb(vec3<f32>(hue, 1.0, 1.0));

    // The first directional light, which is the only one that casts
    // (#734 is the other half). Without one there is nothing to sample
    // against and the atlas check above already answered.
    for (var i = 0u; i < inti.light_count; i = i + 1u) {
        let light = inti_lights[i];
        if (light.kind != INTI_KIND_DIRECTIONAL) {
            continue;
        }
        let s = inti_sample_light(light, world_position);
        let n_dot_l = dot(n, s.to_light);
        if (n_dot_l <= 0.0) {
            // Facing away from the sun. Not shadowed — unlit, which is
            // a different answer and has a different fix.
            return colour * 0.12;
        }
        let lit = inti_sample_cascade(index, world_position, n, s.to_light, n_dot_l);
        return colour * mix(0.30, 1.0, lit);
    }
    return colour * 0.65;
}

/// The contact-shadow march, as colour, for the first light that opted
/// in (#735).
///
/// **One light, because the march is per light**: summing several would
/// average away the thing being looked at. The first opted-in light is
/// the sun in every scene that has one, which is the light whose
/// contact shadow anybody is inspecting.
///
/// The colours are `inti_contact_shadow_debug`'s and the reasoning is
/// there. Magenta here means *no light in the scene marches at all* —
/// which is a different answer from "it marched and found nothing", and
/// they look identical in a shaded frame.
fn inti_contact_shadow_debug_view(
    world_position: vec3<f32>,
    n: vec3<f32>,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    for (var i = 0u; i < inti.light_count; i = i + 1u) {
        let light = inti_lights[i];
        if ((light.flags & INTI_LIGHT_CONTACT_SHADOWS) == 0u) {
            continue;
        }
        let s = inti_sample_light(light, world_position);
        // Same gate the shading loop applies: a surface facing away
        // from the light is not marched, and painting it as "no hit"
        // would read as a failure of the march rather than as geometry.
        if (dot(n, s.to_light) <= 0.0) {
            return vec3<f32>(0.04);
        }
        let to_camera = normalize(inti.camera_position - world_position);
        return inti_contact_shadow_debug(
            inti_contact_shadow_probe(world_position, n, to_camera, s.to_light, frag_coord));
    }
    return vec3<f32>(1.0, 0.0, 1.0);
}

/// `true` when `mode` is one of the views this file draws.
///
/// 🔴 The production build concatenates `INTI_DEBUG_STUB` instead, where
/// this returns a literal `false`. That is what deletes every view above
/// from the game's shader: the call inlines to `if (false)`, and the
/// branch — with its cascade sampling and its screen-space march — is
/// folded away before register allocation ever sees it.
fn inti_debug_is_view(mode: u32) -> bool {
    return mode >= INTI_DEBUG_FIRST;
}

/// The selected view, as colour. Called once, from the one place in each
/// shading path where the surface has just been reconstructed.
fn inti_debug_view(
    mode: u32,
    world_position: vec3<f32>,
    n: vec3<f32>,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    if (mode == INTI_DEBUG_NORMALS) {
        return n * 0.5 + 0.5;
    }
    if (mode == INTI_DEBUG_SHADOW_CASCADES) {
        let view_depth = dot(world_position - inti.camera_position, inti.camera_forward);
        return inti_shadow_debug(world_position, n, view_depth);
    }
    if (mode == INTI_DEBUG_CONTACT_SHADOWS) {
        return inti_contact_shadow_debug_view(world_position, n, frag_coord);
    }
    // A mode the shader does not know. Black rather than a guess: an
    // unimplemented view that renders *something* is one somebody
    // reports as a wrong answer instead of as a missing one.
    return vec3<f32>(0.0);
}
