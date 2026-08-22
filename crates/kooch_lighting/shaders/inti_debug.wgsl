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
const INTI_DEBUG_POINT_SHADOW: u32 = 16u;
const INTI_DEBUG_POINT_CUBE: u32 = 17u;
const INTI_DEBUG_VIRTUAL_PAGES: u32 = 26u;
/// Mirrors `MeshletDebugMode::VirtualPageTiles`. Painted by the MARKING
/// pass, not here — named so the range check lets it through.
const INTI_DEBUG_VIRTUAL_TILES: u32 = 27u;
/// Mirrors `MeshletDebugMode::VirtualPageAge`.
const INTI_DEBUG_VIRTUAL_AGE: u32 = 28u;
/// Mirrors `MeshletDebugMode::LocalPageFaces`.
const INTI_DEBUG_LAMP_FACES: u32 = 29u;
/// Mirrors `MeshletDebugMode::LocalPageDepth`.
const INTI_DEBUG_LAMP_DEPTH: u32 = 30u;
// Lowest discriminant handled here. Modes below it are resolved by the
// shading path itself before the surface is even reconstructed.
const INTI_DEBUG_FIRST: u32 = INTI_DEBUG_NORMALS;
// 🔴 And the highest, which is NOT optional. The dispatch below used to
// be an open-ended `>=`, so every discriminant added above this range
// silently became "an Inti view Inti does not implement" — and the
// fallthrough for those is BLACK. A mode resolved somewhere else
// entirely (the texture mip level in the material shader, FSR's
// intermediates in the upscaler) had its surface painted black before
// the pass that was meant to answer for it ever ran.
const INTI_DEBUG_LAST: u32 = INTI_DEBUG_POINT_CUBE;

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
        surf, inti_lights[inti.debug_light], inti.debug_light, frag_coord);
    // Tonemapped, not raw: the view answers a question about a frame the
    // viewer is looking at, and reading it in a different response curve
    // than that frame reintroduces the ambiguity it exists to remove.
    return inti_tonemap(vec3<f32>(dot(radiance, INTI_LUMA)));
}


/// A point light's cube map, answering for itself (#852).
///
/// # Why a shaded frame cannot answer this
///
/// "The shadow is not there" is four different faults wearing the same
/// pixel: no lamp near this point casts at all, the point is past the
/// lamp's reach so there is nothing to block, the cube says lit because
/// the occluder never reached the map, or the cube says dark and the
/// other lamps in the room fill it back in. Those have four different
/// fixes and a lit frame shows one colour for all of them — which is how
/// a whole session went into a defect that turned out to be two defects
/// and one piece of arithmetic.
///
/// So this paints the cube's answer and NOTHING else. No BRDF, no
/// cosine, no exposure, no ambient, no other light:
///
/// - **magenta** — no point light with a cube reaches this pixel
/// - **blue**    — a lamp holds a cube but this point is past its
///                 `range`, so the map is never consulted
/// - **grey**    — the cube's own factor: black is fully occluded, white
///                 is fully lit, and the ramp between them is the filter
///
/// Which lamp is the one selected in the World panel when that is a
/// casting point light, and otherwise the FIRST one that casts — a
/// choice that is the same for every pixel. See the loop for why it is
/// not the strongest one per pixel.
fn inti_point_shadow_debug(world_position: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let chosen = inti_point_debug_light();
    if (chosen == 0xffffffffu) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }

    let light = inti_lights[chosen];
    let to_light = light.position - world_position;
    let distance_sq = dot(to_light, to_light);
    // The same window `inti_distance_attenuation` saturates. Past it the
    // lamp contributes nothing, so whatever the cube holds is moot and
    // saying "lit" here would read as a hole in the map.
    if (distance_sq >= light.range * light.range) {
        return vec3<f32>(0.0, 0.0, 0.35);
    }

    return vec3<f32>(inti_point_shadow(
        light.shadow_slot,
        world_position,
        n,
        normalize(to_light),
        light.position));
}


/// Which lamp the two point-shadow views answer about, or
/// `0xffffffff` when none casts. Shared so the factor view and the cube
/// view can never disagree about whose shadow is on screen.
fn inti_point_debug_light() -> u32 {
    let selected = inti.debug_light;
    if (selected < inti.light_count
        && inti_lights[selected].kind == INTI_KIND_POINT
        && inti_lights[selected].shadow_slot != INTI_NO_SHADOW_SLOT) {
        return selected;
    }
    for (var i = 0u; i < inti.light_count; i = i + 1u) {
        let light = inti_lights[i];
        if (light.kind == INTI_KIND_POINT
            && light.shadow_slot != INTI_NO_SHADOW_SLOT) {
            return i;
        }
    }
    return 0xffffffffu;
}

/// The cube map itself, all six faces at once (#852).
///
/// The factor view above answers "is this point occluded". When the
/// answer is wrong there are still two possibilities left — the map
/// holds the wrong depth, or it holds nothing at all because the
/// occluder never got rasterised into that face — and only opening the
/// texture separates them.
///
/// The screen becomes a 3x2 grid, one cell per world axis in the order
/// +X, -X, +Y, -Y, +Z, -Z. Which array layer answers is left to the
/// hardware, exactly as it is during shading: the cell builds a world
/// direction and samples with it, so what is on screen is what the
/// shading model would have read looking that way. A face that renders
/// blank here is a face the shading model also finds blank.
///
/// - **dark blue** — nothing recorded: reversed-Z clears to 0, which the
///   comparison reads as "no occluder between here and infinity". This
///   is the picture of an occluder that was culled out of the map.
/// - **grey ramp** — distance to the recorded occluder over the lamp's
///   `range`. Black is at the bulb, white is at the edge of its reach.
/// - **magenta** — no point light casts, or the frame is not clustered
///   (the screen size is derived from the froxel grid).
fn inti_point_cube_debug(frag_coord: vec2<f32>) -> vec3<f32> {
    let chosen = inti_point_debug_light();
    // The grid is the only thing in this uniform that knows how big the
    // screen is: `cluster_factors.xy` is tiles per pixel and
    // `cluster_dimensions.xy` is how many tiles there are.
    if (chosen == 0xffffffffu || inti.clustered == 0u) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    let screen = vec2<f32>(inti.cluster_dimensions.xy)
        / max(inti.cluster_factors.xy, vec2<f32>(1e-6));
    let cell = vec2<f32>(frag_coord.x / screen.x * 3.0, frag_coord.y / screen.y * 2.0);
    let face = u32(clamp(floor(cell.y), 0.0, 1.0)) * 3u
        + u32(clamp(floor(cell.x), 0.0, 2.0));
    // -1..1 inside the cell, with a hairline of margin so the six panels
    // read as six panels rather than as one smear.
    let local = (fract(cell) * 2.0 - 1.0) * 0.97;

    var axis = vec3<f32>(1.0, 0.0, 0.0);
    var right = vec3<f32>(0.0, 0.0, 1.0);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if (face == 1u) {
        axis = vec3<f32>(-1.0, 0.0, 0.0);
    } else if (face == 2u) {
        axis = vec3<f32>(0.0, 1.0, 0.0);
        right = vec3<f32>(1.0, 0.0, 0.0);
        up = vec3<f32>(0.0, 0.0, 1.0);
    } else if (face == 3u) {
        axis = vec3<f32>(0.0, -1.0, 0.0);
        right = vec3<f32>(1.0, 0.0, 0.0);
        up = vec3<f32>(0.0, 0.0, 1.0);
    } else if (face == 4u) {
        axis = vec3<f32>(0.0, 0.0, 1.0);
        right = vec3<f32>(1.0, 0.0, 0.0);
    } else if (face == 5u) {
        axis = vec3<f32>(0.0, 0.0, -1.0);
        right = vec3<f32>(1.0, 0.0, 0.0);
    }

    let light = inti_lights[chosen];
    let record = inti.point_shadows[light.shadow_slot];
    // The same mirror the shading path applies. Sampling the debug view
    // through a different convention than the one being debugged is how
    // an instrument confirms whatever it is pointed at.
    let dir = (axis + right * local.x + up * local.y) * vec3<f32>(1.0, 1.0, -1.0);
    let depth = textureSampleLevel(
        inti_point_cubes, inti_shadow_point_sampler, dir, i32(light.shadow_slot), 0i);
    if (depth <= 0.0) {
        return vec3<f32>(0.05, 0.05, 0.25);
    }
    // Reversed-Z: the stored value is `near / distance_along_major_axis`.
    let metres = record.near / depth;
    return vec3<f32>(clamp(metres / max(light.range, 1e-4), 0.0, 1.0));
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

/// What the virtual shadow pages see at this point, as colour (#866).
///
/// # Three causes that look like one missing shadow
///
/// A hole in a paged shadow is one of three unrelated bugs, and a shaded
/// frame cannot tell them apart:
///
/// - **red** — the walk reached the coarsest level without finding a
///   resident page. Marking and sampling disagree about which page
///   covers this point.
/// - **yellow** — a page IS mapped here and holds the clear value, so
///   nothing was ever rasterised into it. The cull or the expansion
///   dropped the caster for that page.
/// - **green** — a page with real depth, and the comparison says lit.
///   If a caster is visibly overhead, the bias or the depth space is
///   wrong.
///
/// **blue** is a point the pages really do shadow, and **magenta** means
/// the paged path is not running at all — no sun, or the atlas is
/// unbound and the cascades are answering instead.
///
/// Brightness is the clipmap level the answer came from, so the bands
/// stay visible without drowning the classification.
///
/// 🔴 The comparison is the SHADING PASS'S, bias and 2x2 filter
/// included. The first version of this view compared raw, and what it
/// produced was a screenful of green-and-blue moiré with the answer
/// drowned underneath — the identical mistake `inti_shadow_debug`
/// already carries a paragraph about, made two files away from where it
/// is written down. A view whose own noise hides its answer is not a
/// view.
///
/// 🔴 The walk is repeated here rather than shared, the way
/// `inti_shadow_debug` repeats `inti_pick_cascade`: the production
/// function returns one scalar and this needs to know WHY it is that
/// scalar. What is NOT repeated is the lookup, the basis or the page
/// arithmetic — those are the shared functions, so a drift between the
/// view and the thing it describes cannot come from them.
/// Which page the reader lands on, how old it is, and which clipmap
/// level it came from.
///
/// # 🔴 Built to make a FLICKER readable
///
/// A shadow that blinks while the camera moves is four different faults
/// wearing the same coat, and the residency view cannot tell them apart
/// because it answers a still frame. This one answers "what changed":
///
/// - **White** — the page was allocated THIS frame. A sweep of white
///   moving with the camera is the allocator churning; if the flicker
///   rides that sweep, the fault is in allocation.
/// - **Hue** — the clipmap LEVEL the walk stopped at, cycling every six.
///   A band of hue that jumps between two colours frame to frame is the
///   reader crossing a level boundary, which changes the texel size and
///   the rect underneath it. That is a fault in level selection, not in
///   the pool.
/// - **Brightness** — how many frames since the page was last requested,
///   full at one and dim by sixteen. A page dimming while still on
///   screen means marking stopped asking for it while the reader kept
///   finding it.
/// - **Black** — no page at any level. **Magenta** — the paged path is
///   not running.
///
/// The three signals are independent on purpose: the useless version of
/// this view is one colour ramp that every fault can produce.
fn inti_page_age_debug(world_position: vec3<f32>) -> vec3<f32> {
    if (inti.shadows_enabled == 0u || inti_pages.sun.w <= 0.5) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    let basis = sun_basis(inti_pages.sun.xyz);
    let base = inti_pages.world.x;
    let side = inti_pages.space.z;
    let raw = sun_plane(world_position, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let reach = max(abs(raw.x), abs(raw.y)) * 2.0;
    var level = sun_level(reach, base, side);

    for (; level < inti_pages.chain.x; level = level + 1u) {
        let extent = base * exp2(f32(level));
        let centre = sun_centre(inti_pages.eye.xyz, basis, base, side, level);
        let plane = sun_plane(world_position, basis) - centre;
        let uv = clamp(
            plane / extent + vec2<f32>(0.5),
            vec2<f32>(0.0),
            vec2<f32>(0.99999),
        );
        let cell = vec2<u32>(uv * f32(side));
        let page = inti_pages.views.x * inti_pages.views.y
            + inti_pages.space.w * inti_pages.space.x
            + level * side * side
            + cell.y * side
            + cell.x;

        // The probe is walked here rather than through `inti_page_lookup`
        // because the entry INDEX is what carries the age, and the
        // production lookup returns only the slot.
        let entries = inti_pages.pool.x;
        if entries == 0u {
            return vec3<f32>(0.0);
        }
        var probe = page_probe(page, entries);
        var found = false;
        for (var i = 0u; i < PAGE_PROBES; i = i + 1u) {
            let key = inti_page_keys[probe];
            if key == PAGE_EMPTY {
                break;
            }
            if key == page + 1u {
                found = true;
                break;
            }
            probe = page_step(probe, entries);
        }
        if !found {
            continue;
        }

        let age = inti_page_slots[probe * PAGE_CELL + 1u];
        let since = inti_pages.views.w - age;
        // Allocated this frame. White, and deliberately the loudest
        // thing on screen: it is the signal a flicker has to be
        // correlated against.
        if since == 0u {
            return vec3<f32>(1.0);
        }

        var hue = vec3<f32>(0.6);
        switch level % 6u {
            case 0u: { hue = vec3<f32>(1.0, 0.25, 0.25); }
            case 1u: { hue = vec3<f32>(1.0, 0.65, 0.2); }
            case 2u: { hue = vec3<f32>(0.9, 0.95, 0.25); }
            case 3u: { hue = vec3<f32>(0.3, 0.9, 0.4); }
            case 4u: { hue = vec3<f32>(0.3, 0.6, 1.0); }
            default: { hue = vec3<f32>(0.75, 0.4, 1.0); }
        }
        // Full at one frame, a fifth by sixteen.
        let fade = clamp(1.0 - f32(since - 1u) / 16.0, 0.2, 1.0);
        return hue * fade;
    }
    // Nothing at any level.
    return vec3<f32>(0.0);
}

fn inti_page_debug(world_position: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    // The same term the shading pass feeds the bias, so a surface at a
    // grazing angle is judged the way the frame judges it.
    let n_dot_l = clamp(dot(n, -normalize(inti_pages.sun.xyz)), 0.0, 1.0);
    if (inti.shadows_enabled == 0u || inti_pages.sun.w <= 0.5) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    let basis = sun_basis(inti_pages.sun.xyz);
    let base = inti_pages.world.x;
    let side = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;
    let levels = inti_pages.chain.x;

    let raw = sun_plane(world_position, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let reach = max(abs(raw.x), abs(raw.y)) * 2.0;
    var level = sun_level(reach, base, side);

    for (; level < levels; level = level + 1u) {
        let extent = base * exp2(f32(level));
        let centre = sun_centre(inti_pages.eye.xyz, basis, base, side, level);
        let plane = sun_plane(world_position, basis) - centre;
        let uv = clamp(
            plane / extent + vec2<f32>(0.5),
            vec2<f32>(0.0),
            vec2<f32>(0.99999),
        );
        let cell = vec2<u32>(uv * f32(side));
        let page = inti_pages.views.x * inti_pages.views.y
            + inti_pages.space.w * inti_pages.space.x
            + level * side * side
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if (slot == PAGE_MISS) {
            continue;
        }
        // Level as brightness, so the clipmap bands read at a glance.
        let shade = 0.45 + 0.55 * f32(levels - min(level, levels - 1u)) / f32(max(levels, 1u));

        let rect = sun_page_rect(level, cell, base, side, centre);
        let within = (sun_plane(world_position, basis) - rect.xy) / rect.z + vec2<f32>(0.5);
        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let texel = clamp(
            floor(within * f32(page_texels)),
            vec2<f32>(0.0),
            vec2<f32>(f32(page_texels) - 1.0),
        );
        let at = vec2<i32>(vec2<f32>(place.xy) + texel);
        let stored = textureLoad(inti_page_atlas, at, i32(place.z), 0);

        // Reversed-Z: the pass clears to 0 and 0 is FAR, so a page that
        // nothing drew into reads exactly 0 everywhere.
        if (stored <= 0.0) {
            return vec3<f32>(shade, shade, 0.0);
        }
        // 🔴 `inti_page_shadow`'s own answer, not a second comparison —
        // bias, filter and all. Anything else measures a shadow this
        // engine does not draw.
        let lit = inti_page_shadow(world_position, n, -normalize(inti_pages.sun.xyz), n_dot_l);
        return mix(vec3<f32>(0.0, 0.0, shade), vec3<f32>(0.0, shade, 0.0), lit);
    }
    return vec3<f32>(1.0, 0.0, 0.0);
}

/// 🔴 The production build concatenates `INTI_DEBUG_STUB` instead, where
/// this returns a literal `false`. That is what deletes every view above
/// from the game's shader: the call inlines to `if (false)`, and the
/// branch — with its cascade sampling and its screen-space march — is
/// folded away before register allocation ever sees it.
fn inti_debug_is_view(mode: u32) -> bool {
    // 🔴 The virtual-page view is named, not folded into the range.
    // Stretching `INTI_DEBUG_LAST` up to 26 would swallow 18 through 25
    // — the texture mip level and every FSR intermediate — and those are
    // answered by other passes entirely. Claiming them here paints their
    // surface BLACK before the pass that owns them ever runs, which is
    // the exact failure the comment on `INTI_DEBUG_LAST` exists for.
    return (mode >= INTI_DEBUG_FIRST && mode <= INTI_DEBUG_LAST)
        || mode == INTI_DEBUG_VIRTUAL_PAGES
        || mode == INTI_DEBUG_VIRTUAL_AGE
        || mode == INTI_DEBUG_LAMP_FACES
        || mode == INTI_DEBUG_LAMP_DEPTH;
}

/// One lamp's shadow pages, taken apart.
///
/// # 🔴 Why the lamp is FIXED and why there are two views
///
/// A lamp's page arithmetic carries six sign choices the sun's does not
/// — one per cube face — and every one is invisible in the shaded
/// image. Painting all hundred lamps at once averages exactly the signal
/// being looked for, so this reads `debug_light` and answers about that
/// one.
///
/// `faces` picks which question: the face and level a pixel READS
/// (`true`), or what that page CONTAINED (`false`). A shadow that looks
/// wrong is either reading the wrong page or reading the right page and
/// comparing wrong, and no single view can separate those.
///
/// Deliberately a SECOND walk of the chain rather than a hook inside
/// `inti_local_page_shadow`: a debug view that shares the reader's early
/// returns cannot show what the reader skipped.
fn inti_lamp_page_debug(world_position: vec3<f32>, n: vec3<f32>, faces: bool) -> vec3<f32> {
    if (inti.shadows_enabled == 0u || inti_pages.sun.w <= 0.5) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    // 🔴 ORANGE, not magenta. Three different reasons to show nothing
    // wearing one colour is how a whole view reads as broken: "the
    // paged path is off" and "you have not picked a lamp" have
    // different fixes and the second one is a click. Select a point or
    // spot light in the World panel.
    let light_index = inti.debug_light;
    if (light_index >= inti.light_count) {
        return vec3<f32>(1.0, 0.55, 0.1);
    }
    let light = inti_lights[light_index];
    if (light.kind == INTI_KIND_DIRECTIONAL) {
        return vec3<f32>(1.0, 0.55, 0.1);
    }

    let raw = world_position - light.position;
    let distance = length(raw);
    // Out of the lamp's reach: nothing to say, and saying something
    // would fill the screen with a colour that means "not applicable".
    if (distance > light.range) {
        return vec3<f32>(0.0);
    }
    let to_light = -raw / max(distance, 1e-6);
    let is_spot = light.kind == INTI_KIND_SPOT;

    let side0 = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;
    let stride = inti_pages.space.x;
    let face_pages = inti_pages.space.y;
    let levels = u32(log2(f32(max(side0, 1u)))) + 1u;
    let view_base = inti_pages.views.x * inti_pages.views.y;

    for (var level = 0u; level < levels; level = level + 1u) {
        let side = level_side_of(level, side0);
        let texel_world = 2.0 * max(distance, PAGE_NEAR) / f32(side * page_texels);
        let sampled = world_position
            + n * (texel_world * INTI_NORMAL_BIAS)
            + to_light * INTI_DEPTH_BIAS;
        let offset = sampled - light.position;
        let hit = cube_face(offset);
        let face = select(u32(hit.w), 0u, is_spot);
        let cell = vec2<u32>(
            clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side)
        );
        let page = view_base
            + light_index * stride
            + face * face_pages
            + level_base_of(level, side0)
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if (slot == PAGE_MISS) {
            continue;
        }

        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let step = 1.0 / f32(side);
        let within = (hit.xy - vec2<f32>(cell) * step) / step;
        let texel = clamp(
            floor(within * f32(page_texels)),
            vec2<f32>(0.0),
            vec2<f32>(f32(page_texels) - 1.0),
        );
        let stored = textureLoad(
            inti_page_atlas, vec2<i32>(vec2<f32>(place.xy) + texel), i32(place.z), 0);
        let receiver = clamp(PAGE_NEAR / max(length(offset), PAGE_NEAR), 0.0, 1.0);

        if (!faces) {
            // Red occluded, green lit. One tap, not the reader's 2x2:
            // a filtered answer cannot say which texel disagreed.
            return select(vec3<f32>(0.1, 0.9, 0.1), vec3<f32>(0.9, 0.1, 0.1), stored > receiver);
        }
        // Six hues, evenly spaced, so no two adjacent faces share one.
        var hue = vec3<f32>(0.0);
        switch face {
            case 0u: { hue = vec3<f32>(1.0, 0.25, 0.25); }
            case 1u: { hue = vec3<f32>(0.25, 1.0, 0.25); }
            case 2u: { hue = vec3<f32>(0.25, 0.4, 1.0); }
            case 3u: { hue = vec3<f32>(1.0, 1.0, 0.3); }
            case 4u: { hue = vec3<f32>(1.0, 0.35, 1.0); }
            default: { hue = vec3<f32>(0.3, 1.0, 1.0); }
        }
        // Level as brightness, finest brightest, so the chain bands read
        // without competing with the face hue.
        let shade = 0.4 + 0.6 * f32(levels - min(level, levels - 1u)) / f32(max(levels, 1u));
        return hue * shade;
    }

    // The lamp reaches here and no level held a page. WHITE for the face
    // view and BLUE for the depth one — the distinction that separates a
    // page never allocated from a page that answered wrong.
    return select(vec3<f32>(0.2, 0.3, 1.0), vec3<f32>(1.0), faces);
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
    if (mode == INTI_DEBUG_POINT_SHADOW) {
        return inti_point_shadow_debug(world_position, n);
    }
    if (mode == INTI_DEBUG_POINT_CUBE) {
        return inti_point_cube_debug(frag_coord);
    }
    if (mode == INTI_DEBUG_VIRTUAL_AGE) {
        return inti_page_age_debug(world_position);
    }
    if (mode == INTI_DEBUG_LAMP_FACES) {
        return inti_lamp_page_debug(world_position, n, true);
    }
    if (mode == INTI_DEBUG_LAMP_DEPTH) {
        return inti_lamp_page_debug(world_position, n, false);
    }
    if (mode == INTI_DEBUG_VIRTUAL_PAGES) {
        return inti_page_debug(world_position, n);
    }
    // A mode the shader does not know. Black rather than a guess: an
    // unimplemented view that renders *something* is one somebody
    // reports as a wrong answer instead of as a missing one.
    return vec3<f32>(0.0);
}
