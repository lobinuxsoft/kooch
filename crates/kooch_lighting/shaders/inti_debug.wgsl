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
const INTI_DEBUG_SINGLE_LIGHT: u32 = 14u;
const INTI_DEBUG_LIGHT_COUNT: u32 = 15u;
// Lowest discriminant handled here. Modes below it are resolved by the
// shading path itself before the surface is even reconstructed.
const INTI_DEBUG_FIRST: u32 = INTI_DEBUG_NORMALS;

// Rec. 709 luma weights, applied to LINEAR radiance — which is what
// makes the grey mean "how much light landed here" rather than "how
// bright the pixel ended up".
const INTI_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// The stand-in material the single-light view shades with: a plain
// dielectric, mid-rough.
//
// Roughness is kept rather than zeroed because the width of a highlight
// is information about the LIGHT — a small source and a broad one differ
// there and nowhere else. Metallic is forced off because a metal takes
// its F0 from its albedo, and the albedo is exactly what this view
// removes; a metal shaded with white albedo is not that metal with the
// colour turned off, it is a mirror.
const INTI_DEBUG_ROUGHNESS: f32 = 0.5;

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

/// One light, alone, in grey, with whatever shadow it actually casts
/// (#743).
///
/// # What is removed, and why each one
///
/// - **Every other light.** The question is *why is this dark*, and with
///   two lights in the sum a surface lit by the wrong one still looks
///   lit.
/// - **The material's colour.** A dark albedo and no light reaching the
///   surface produce the same pixel. Shading a neutral white dielectric
///   makes the image a picture of the light instead of a picture of the
///   paint. See `INTI_DEBUG_ROUGHNESS` for what is deliberately kept.
/// - **Ambient.** It belongs to no light, and including it would mean a
///   point in full shadow never renders black — which is precisely the
///   reading this view exists to make unambiguous.
///
/// # What is kept
///
/// The shadow, by calling `inti_light_contribution` — the same function
/// the shading pass sums per light, with its cascade sampling, its bias
/// and its contact-shadow march. A debug view that recomputes the maths
/// its own way can disagree with the frame, and then it is one more
/// thing to debug rather than the thing that ends the argument.
///
/// ⚠️ Only a directional light casts a cascade shadow today, and contact
/// shadows are opt-in and off by default on point and spot. So a punctual
/// light usually renders here with no shadow at all — that is the truth
/// about the engine, not a failure of the view, and the editor says so
/// in words next to the selector rather than leaving it to be guessed.
///
/// Magenta means no light is selected, or the selected entity is not a
/// light in this frame's buffer.
fn inti_single_light_debug(
    world_position: vec3<f32>,
    n: vec3<f32>,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    if (inti.debug_light >= inti.light_count) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    // Always a shadow receiver: the view answers "what does this light
    // do here", and a surface opted out of shadows would answer a
    // different question (#804).
    let surf = inti_surface(
        world_position, n, vec3<f32>(1.0), 0.0, INTI_DEBUG_ROUGHNESS,
        INTI_SURFACE_RECEIVES_SHADOWS);
    let radiance = inti_light_contribution(
        surf, inti_lights[inti.debug_light], frag_coord);
    // Tonemapped, not raw: the view answers a question about a frame the
    // viewer is looking at, and reading it in a different response curve
    // than that frame reintroduces the ambiguity it exists to remove.
    return inti_tonemap(vec3<f32>(dot(radiance, INTI_LUMA)));
}

/// `true` when `mode` is one of the views this file draws.
///
// How many lights this pixel evaluates, as a heatmap (#817).
//
// 🔴 The count is read where it is PAID. `inti_clustered_lights` walks
// exactly `point_count + spot_count` entries of this fragment's cell and
// nothing else, so the same two fields that bound that loop are what
// this view paints. A count assembled from anywhere else — the scene's
// light total, the grid's capacity, a CPU-side estimate — would be a
// second opinion about a number the shader already knows, and the two
// would drift.
//
// Directional lights are added because the grid does not cluster them:
// they reach every cell, are the light buffer's leading entries, and the
// shading loop pays for all of them at every pixel.
fn inti_light_count_debug(world_position: vec3<f32>, frag_coord: vec2<f32>) -> vec3<f32> {
    var count = inti.directional_count;
    if (inti.clustered == 0u) {
        // No grid this frame: every light for every pixel. Flat maximum
        // is the honest answer, not a special case — see the mode's doc
        // comment in `debug.rs`.
        count = inti.light_count;
    } else {
        let cell = inti_clusters[inti_cluster_of(world_position, frag_coord)];
        count = count + cell.point_count + cell.spot_count;
    }
    if (count == 0u) {
        // Black, and deliberately not the ramp's cold end: "no light
        // reaches here" and "one light reaches here" are different
        // answers and the whole view exists to separate them.
        return vec3<f32>(0.0);
    }
    // The top of scale comes from the uniform, not from a constant: the
    // value that separates a busy froxel from a quiet one in a
    // hundred-light stress test washes a four-lamp room flat red. The
    // editor owns it and prints what it is.
    let hot = f32(max(inti.debug_lights_hot, 1u));
    let t = clamp(f32(count) / hot, 0.0, 1.0);
    return inti_count_heatmap(t);
}

// Blue → green → red, the same ramp `density_heatmap` paints in
// `meshlet_debug_resolve.wgsl`.
//
// ⚠️ A second copy, on purpose: the two live in different crates and are
// concatenated into different shaders, and a shared file would exist
// only to hold four clamps. What must not drift is the *reading* — a
// green pixel meaning the middle of the scale in one heatmap and
// something else in another is how an artist learns to distrust both.
fn inti_count_heatmap(t: f32) -> vec3<f32> {
    let r = clamp(2.0 * t - 1.0, 0.0, 1.0);
    let g = clamp(1.0 - 2.0 * abs(t - 0.5), 0.0, 1.0);
    let b = clamp(1.0 - 2.0 * t, 0.0, 1.0);
    return vec3<f32>(r, g, b);
}

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
    if (mode == INTI_DEBUG_SINGLE_LIGHT) {
        return inti_single_light_debug(world_position, n, frag_coord);
    }
    if (mode == INTI_DEBUG_LIGHT_COUNT) {
        return inti_light_count_debug(world_position, frag_coord);
    }
    // A mode the shader does not know. Black rather than a guess: an
    // unimplemented view that renders *something* is one somebody
    // reports as a wrong answer instead of as a missing one.
    return vec3<f32>(0.0);
}
