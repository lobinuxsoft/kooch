// Debug views, concatenated after `inti_pbr.wgsl` only in a debug pipeline. An untaken branch still
// raises VGPR count (worst-case register allocation), which caps a 10 W iGPU; production gets
// `INTI_DEBUG_STUB`.

// `MeshletDebugMode` discriminants, pinned by a test in `kooch_render`'s `debug.rs`; one copy so
// R64 and R32 cannot mean different modes.
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
// 🔴 The upper bound is required: an open-ended `>=` claimed every newer mode and painted it black
// before the pass owning it (mip level, FSR intermediates) ran.
const INTI_DEBUG_LAST: u32 = INTI_DEBUG_POINT_CUBE;

// Rec. 709 luma weights, applied to LINEAR radiance — which is what
// makes the grey mean "how much light landed here" rather than "how
// bright the pixel ended up".
const INTI_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// The single-light view's material: dielectric, mid-rough. Roughness stays because highlight width
// says something about the light; metallic goes because a metal with no albedo is a mirror.
const INTI_DEBUG_ROUGHNESS: f32 = 0.5;

// Bevy's cascade hues (`shadows.wgsl:265`) and constants, so captures read the same; dividing by
// count + 1 keeps the last cascade off the first one's colour.
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

/// Cascade hue as Bevy's, dimmed where `inti_sample_cascade` — the shading call — shadows (#476); a
/// raw sample drowned it in moiré. Magenta no atlas · black no cascade · grey past the last ·
/// bright lit · dim shadowed.
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

/// The contact-shadow march for the first opted-in light (#735) — one, since summing averages it
/// away. Magenta means no light marches, unlike "marched and found nothing".
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

/// One light in grey with its real shadow (#743), via `inti_light_contribution` so it matches the
/// frame; other lights, albedo and ambient removed. Punctual lights often cast nothing — the editor
/// says so. Magenta: none selected.
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


/// A point light's cube answering alone (#852), no BRDF or other lights, since four faults share
/// one dark pixel. Magenta no casting lamp here · blue past its `range` · grey the cube's factor.
/// The lamp is the selected casting point light, else the first — the same for every pixel.
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

/// All six cube faces as a 3×2 grid (+X −X +Y −Y +Z −Z), sampled by direction as shading does
/// (#852). Dark blue nothing recorded (a culled occluder) · grey distance over `range` · magenta no
/// caster or no grid.
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

/// Lights this pixel evaluates, as a heatmap (#817). 🔴 Read from the two counts that bound
/// `inti_clustered_lights`, where it is paid; directional lights are added since every pixel pays
/// for them.
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
    // Top of scale from the uniform: a hundred-light stress test's value washes a four-lamp room
    // red. The editor owns and prints it.
    let hot = f32(max(inti.debug_lights_hot, 1u));
    let t = clamp(f32(count) / hot, 0.0, 1.0);
    return inti_count_heatmap(t);
}

// Blue → green → red, the ramp `density_heatmap` paints. A copy on purpose, across crates; what
// must not drift is what a colour means.
fn inti_count_heatmap(t: f32) -> vec3<f32> {
    let r = clamp(2.0 * t - 1.0, 0.0, 1.0);
    let g = clamp(1.0 - 2.0 * abs(t - 0.5), 0.0, 1.0);
    let b = clamp(1.0 - 2.0 * t, 0.0, 1.0);
    return vec3<f32>(r, g, b);
}

/// Page age (#866), to read a flicker: white no content · hue the clipmap level · brightness frames
/// since allocation, dim by 16 · black no page · magenta paging off. Independent signals, not one
/// ramp any fault fills.
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
        // The same absolute-world key the marking wrote; a debug view on
        // the old camera-relative one would paint the wrong page.
        let cell = sun_cell(world_position, inti_pages.eye.xyz, basis, base, side, level);
        let page = inti_pages.views.x * inti_pages.views.y
            + inti_pages.space.w * inti_pages.space.x
            + level * side * side
            + cell.y * side
            + cell.x;

        // Indexed here rather than through `inti_page_lookup` because
        // the AGE word is what this view paints, and the production
        // lookup returns only the slot.
        if page >= inti_pages.pool.x {
            return vec3<f32>(0.0);
        }
        if inti_page_slots[page * PAGE_CELL] == PAGE_ABSENT {
            continue;
        }

        // 🔴 White is a page with no content (word 3 is zero until drawn). It used to be "requested
        // this frame", which is every visible page, so the whole screen went white.
        if inti_page_slots[page * PAGE_CELL + 3u] == 0u {
            return vec3<f32>(1.0);
        }
        // Frames since allocation — word 5, written only by `page_stamp` (word 1 is refreshed every
        // frame). ⚠️ Unsigned, so the subtraction saturates instead of wrapping to four billion.
        let born = inti_page_slots[page * PAGE_CELL + 5u];
        let since = select(0u, inti_pages.views.w - born, inti_pages.views.w >= born);

        var hue = vec3<f32>(0.6);
        switch level % 6u {
            case 0u: { hue = vec3<f32>(1.0, 0.25, 0.25); }
            case 1u: { hue = vec3<f32>(1.0, 0.65, 0.2); }
            case 2u: { hue = vec3<f32>(0.9, 0.95, 0.25); }
            case 3u: { hue = vec3<f32>(0.3, 0.9, 0.4); }
            case 4u: { hue = vec3<f32>(0.3, 0.6, 1.0); }
            default: { hue = vec3<f32>(0.75, 0.4, 1.0); }
        }
        // Full when allocated, a fifth by sixteen frames: a bright sweep with the camera is the
        // allocator churning.
        let fade = clamp(1.0 - f32(since) / 16.0, 0.2, 1.0);
        return hue * fade;
    }
    // Nothing at any level.
    return vec3<f32>(0.0);
}

/// Virtual page residency (#866): red no resident page at any level · yellow mapped but never drawn
/// · green real depth that compares lit · blue shadowed · magenta paging off. The comparison is the
/// shading pass's.
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
        // The same absolute-world key the marking wrote; a debug view on
        // the old camera-relative one would paint the wrong page.
        let cell = sun_cell(world_position, inti_pages.eye.xyz, basis, base, side, level);
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

        let rect = sun_page_rect(level, cell, inti_pages.eye.xyz, basis, base, side);
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

/// `true` when `mode` is one of these views. 🔴 The production stub returns literal `false`, folding
/// every view away before register allocation.
fn inti_debug_is_view(mode: u32) -> bool {
    // 🔴 The virtual-page view is named, not added to the range: stretching `INTI_DEBUG_LAST` to 26
    // would swallow 18–25 and paint their surfaces black.
    return (mode >= INTI_DEBUG_FIRST && mode <= INTI_DEBUG_LAST)
        || mode == INTI_DEBUG_VIRTUAL_PAGES
        || mode == INTI_DEBUG_VIRTUAL_AGE
        || mode == INTI_DEBUG_LAMP_FACES
        || mode == INTI_DEBUG_LAMP_DEPTH;
}

/// One lamp's pages taken apart, for `debug_light` only — painting all lamps averages away six
/// per-face sign choices. `faces`: which face and level a pixel reads, or what that page held.
/// A second walk of the chain, since sharing the reader's early returns hides what it skipped.
fn inti_lamp_page_debug(world_position: vec3<f32>, n: vec3<f32>, faces: bool) -> vec3<f32> {
    // Gated on pages being bound, not `shadows_enabled` — that is the cascade flag, 0 in any scene
    // without a sun.
    if (inti_pages.sun.w <= 0.5) {
        return vec3<f32>(1.0, 0.0, 1.0);
    }
    // 🔴 Orange, not magenta: "paging is off" and "no lamp selected" have different fixes, and the
    // second is a click.
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

    for (var level = local_level_floor(side0 * page_texels); level < levels; level = level + 1u) {
        let side = level_side_of(level, side0);
        let texel_world = 2.0 * max(distance, PAGE_NEAR) / f32(side * page_texels);
        // 🔴 The face view reads the raw position and the occlusion view the biased one: the bias
        // crosses face seams and would draw seams that are not the cube's.
        var sampled = world_position;
        if (!faces) {
            sampled = world_position
                + n * (texel_world * INTI_POINT_NORMAL_BIAS)
                + to_light * INTI_POINT_DEPTH_BIAS;
        }
        var offset = sampled - light.position;
        if (is_spot) {
            offset = spot_local(light.direction, offset);
        }
        let hit = cube_face(offset);
        let face = select(u32(hit.w), 0u, is_spot);
        let cell = vec2<u32>(
            clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side)
        );
        let page = view_base
            + light_index * stride
            + face * face_pages
            + local_level_base(level, side0, page_texels)
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
        let major = max(max(abs(offset.x), abs(offset.y)), abs(offset.z));
        let receiver = clamp(PAGE_NEAR / max(major, PAGE_NEAR), 0.0, 1.0);

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
