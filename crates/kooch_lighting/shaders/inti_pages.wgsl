/// Where a virtual page lives, or `PAGE_MISS` — one indexed read (#477); the hash walk it replaced
/// cost 10.4 ms of shading against 0.884 ms for the whole shadow track.
fn inti_page_lookup(page: u32) -> u32 {
    if page >= inti_pages.pool.x {
        return PAGE_MISS;
    }
    let stored = inti_page_slots[page * PAGE_CELL];
    if stored == PAGE_ABSENT {
        return PAGE_MISS;
    }
    // 🔴 Resident is not readable: stamp 0 means no content and the atlas answers lit. A miss lets
    // the walk climb, as Unreal; an older stamp still reads, and `PAGE_EMPTY` is a true "cleared".
    if inti_page_slots[page * PAGE_CELL + 3u] == 0u {
        return PAGE_MISS;
    }
    return stored - 1u;
}

/// Page PCF by hand, since hardware filters cross page edges: taps clamp to the page, weights cross
/// it. `world.w` is the box width (#941): 1 bilinear, wider a sub-texel box of `(W + 1)²` loads.
fn inti_page_filter(
    origin: vec2<f32>,
    layer: i32,
    texel: vec2<f32>,
    receiver: f32,
    // The receiver's depth gradient per texel (#1017). Zero puts every tap back on the pixel's own
    // depth, which is what a scalar bias assumes and what the lamps still pass.
    slope: vec2<f32>,
    page_texels: u32,
    // The page's sun-clipmap cell, so a tap off its edge finds its neighbour; `PAGE_UNLISTED` level
    // clamps instead (lamps).
    cell: vec2<u32>,
    level: u32,
    side: u32,
) -> f32 {
    let width = max(u32(inti_pages.world.w), 1u);
    let half = f32(width) * 0.5;
    let last = f32(page_texels) - 1.0;
    let corner = floor(texel - vec2<f32>(half));
    let frac = texel - vec2<f32>(half) - corner;
    var lit = 0.0;
    for (var y = 0u; y <= width; y = y + 1u) {
        var wy = 1.0;
        if y == 0u {
            wy = 1.0 - frac.y;
        } else if y == width {
            wy = frac.y;
        }
        for (var x = 0u; x <= width; x = x + 1u) {
            var wx = 1.0;
            if x == 0u {
                wx = 1.0 - frac.x;
            } else if x == width {
                wx = frac.x;
            }
            // 🔴 A tap leaving the page resolves through the table, as Unreal's
            // `VirtualToPhysicalTexel` per tap. Clamping read this page near seams and answered lit
            // — a band along every seam. Absent neighbour: clamp.
            let raw = corner + vec2<f32>(f32(x), f32(y));
            let outside = raw.x < 0.0 || raw.y < 0.0 || raw.x > last || raw.y > last;
            var tap = clamp(raw, vec2<f32>(0.0), vec2<f32>(last));
            var at = vec2<i32>(origin + tap);
            var tap_layer = layer;
            // 🔴 Separate from `tap`, which restarts inside a neighbouring page; the gradient needs
            // the sample's distance from the pixel, or seams draw a grey line.
            var reach = tap;
            if outside && level != PAGE_UNLISTED {
                // Which page the tap fell into, in whole pages, and the wrapped cell it lands on —
                // the same toroidal grid `sun_cell` keys by.
                let step = vec2<i32>(floor(raw / f32(page_texels)));
                let wide = i32(side);
                let neighbour = vec2<u32>(
                    (vec2<i32>(cell) + step + vec2<i32>(wide, wide)) % vec2<i32>(wide, wide),
                );
                let page = inti_pages.views.x * inti_pages.views.y
                    + inti_pages.space.w * inti_pages.space.x
                    + level * side * side
                    + neighbour.y * side
                    + neighbour.x;
                let found = inti_page_lookup(page);
                if found != PAGE_MISS {
                    let place = page_place(
                        found,
                        inti_pages.views.z,
                        inti_pages.pool.z,
                        page_texels,
                    );
                    tap = raw - vec2<f32>(step) * f32(page_texels);
                    at = vec2<i32>(vec2<f32>(place.xy) + tap);
                    tap_layer = i32(place.z);
                    // The sample really is `raw` texels from the corner
                    // of the ORIGINAL page, whatever page holds it.
                    reach = raw;
                }
            }
            let stored = textureLoad(inti_page_atlas, at, tap_layer, 0);
            // 🔴 The tap's own receiver depth, `dot(slope, k)` along the plane — comparing against
            // the pixel's makes tilted surfaces self-shadow. See `receiver_slope`.
            let here = receiver + dot(slope, reach - texel);
            // Reversed-Z: a LARGER stored depth is closer to the light, so it is an occluder.
            let hit = select(1.0, 0.0, stored > here);
            lit = lit + hit * wx * wy;
        }
    }
    return lit / (f32(width) * f32(width));
}

/// One probe of the sun's atlas: `x` stored depth, `y` this position's depth in the same encoding,
/// `z` 1 when a page was found. Shared by the walk and the march.
fn inti_page_read(p: vec3<f32>, basis: mat3x3<f32>) -> vec3<f32> {
    let base = inti_pages.world.x;
    let span = inti_pages.world.y;
    let side = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;

    let raw = sun_plane(p, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let reach = max(abs(raw.x), abs(raw.y)) * 2.0;
    var level = sun_level(reach, base, side);

    for (; level < inti_pages.chain.x; level = level + 1u) {
        let along = dot(p - inti_pages.eye.xyz, basis[2])
            + sun_drift(inti_pages.eye.xyz, basis, base, side, level);
        let mine = 1.0 - (along + span) / (2.0 * span);

        let cell = sun_cell(p, inti_pages.eye.xyz, basis, base, side, level);
        let page = inti_pages.views.x * inti_pages.views.y
            + inti_pages.space.w * inti_pages.space.x
            + level * side * side
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if slot == PAGE_MISS {
            continue;
        }
        let rect = sun_page_rect(level, cell, inti_pages.eye.xyz, basis, base, side);
        let within = (sun_plane(p, basis) - rect.xy) / rect.z + vec2<f32>(0.5);
        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let texel = clamp(
            floor(within * f32(page_texels)),
            vec2<f32>(0.0),
            vec2<f32>(f32(page_texels) - 1.0),
        );
        let at = vec2<i32>(vec2<f32>(place.xy) + texel);
        let stored = textureLoad(inti_page_atlas, at, i32(place.z), 0);
        return vec3<f32>(stored, mine, 1.0);
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

/// Rays over the sun's disc. 🔴 One ray is useless: stepping along the sun's axis stays in the same
/// texel, so the rays must open across the source's angular size.
const PAGE_RAYS: u32 = 4u;
const PAGE_STEPS: u32 = 8u;

/// Occlusion by marching rays across the sun's disc: one tap misses occluders outside its texel.
/// Tolerance is the ray's own depth change per step — no bias constant, as Unreal.
fn inti_page_march(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    n_dot_l: f32,
    jitter: f32,
) -> f32 {
    let basis = sun_basis(inti_pages.sun.xyz);
    let base = inti_pages.world.x;
    let side = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;
    // Along the sun's axis, towards it. `basis[2]` points the way the
    // light travels, so the ray runs against it.
    let to_light = -basis[2];

    // 🔴 Ray reach from the level's extent, not the 2000 m orthographic span, whose quadratic steps
    // put the first sample 60 m away.
    let raw = sun_plane(world_position, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let level = sun_level(max(abs(raw.x), abs(raw.y)) * 2.0, base, side);
    let extent = base * exp2(f32(level));
    let reach = extent;
    // One texel of that level — the unit every other bias here is in.
    let texel_world = extent / f32(side * page_texels);

    // The sun's angular radius, as the tangent of the half-angle. Zero makes every ray identical
    // and the march degenerate, so it has a floor: a disc that small is a hard shadow either way.
    let spread = max(inti.sun_softness, 1e-3);

    // Off the surface by the same texel multiple the box reader uses,
    // so the first samples do not read the receiver itself.
    let start = world_position + normal * (texel_world * inti_pages.bias.x)
        + to_light * inti_pages.bias.y;

    var lit = 0.0;
    for (var r = 0u; r < PAGE_RAYS; r = r + 1u) {
        // Golden-angle spiral over the disc, rotated per pixel so the
        // pattern does not print itself onto flat ground.
        let t = (f32(r) + 0.5) / f32(PAGE_RAYS);
        let angle = f32(r) * 2.39996323 + jitter * 6.2831853;
        let radius = sqrt(t) * spread;
        let dir = normalize(
            to_light + basis[0] * (cos(angle) * radius) + basis[1] * (sin(angle) * radius),
        );

        var blocked = false;
        var previous = -1.0;
        for (var i = 1u; i <= PAGE_STEPS; i = i + 1u) {
            // Quadratic in the step index: dense near the receiver, where contact shadows live and
            // where a miss is most visible, sparse further out.
            let f = f32(i) / f32(PAGE_STEPS);
            let at = start + dir * (f * f * reach);
            let read = inti_page_read(at, basis);
            if read.z == 0.0 {
                continue;
            }
            let reference = read.y;
            if previous >= 0.0 {
                // 🔴 Tolerance from the geometry; the 1.05 slack is Unreal's, or precision makes
                // shadowed regions sparkle.
                let tolerance = abs(reference - previous) * 1.05;
                // 🔴 Reversed-Z: blocked when stored depth exceeds the sample's. Inverted, every ray
                // climbed past its own ground and the frame went uniformly dark.
                if read.x - reference > tolerance {
                    blocked = true;
                    break;
                }
            }
            previous = reference;
        }
        if !blocked {
            lit = lit + 1.0;
        }
    }
    return lit / f32(PAGE_RAYS);
}


/// Sun shadow from the page pool: walks out from the coarsest containing level, since any resident
/// page holds valid depth. Bias moves the position by texels of the level read — metres would
/// vanish far, explode near.
fn inti_page_shadow(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    n_dot_l: f32,
) -> f32 {
    // Which reader answers, from `shadow_page_march`. See
    // `inti_page_march` for what the two ask differently.
    if inti_pages.layer.z != 0u {
        return inti_page_march(world_position, normal, n_dot_l, 0.0);
    }
    let basis = sun_basis(inti_pages.sun.xyz);

    let base = inti_pages.world.x;
    let span = inti_pages.world.y;
    let side = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;

    // Containment is the level floor, measured before the offset: a one-texel step cannot cross a
    // page boundary. Mirrors `mark_sun`'s `contain`.
    let raw = sun_plane(world_position, basis) - sun_plane(inti_pages.eye.xyz, basis);
    let reach = max(abs(raw.x), abs(raw.y)) * 2.0;
    var level = sun_level(reach, base, side);

    // 🔴 Jump instead of walk: `cs_lod_offsets` stored how many levels up the first readable page is
    // (Unreal's `LODOffset`). The loop stays for stale hints.
    let floor_cell = sun_cell(world_position, inti_pages.eye.xyz, basis, base, side, level);
    let floor_page = inti_pages.views.x * inti_pages.views.y
        + inti_pages.space.w * inti_pages.space.x
        + level * side * side
        + floor_cell.y * side
        + floor_cell.x;
    if floor_page < inti_pages.pool.x {
        let hint = inti_page_slots[floor_page * PAGE_CELL + PAGE_LOD];
        if hint != PAGE_NO_LOD && level + hint < inti_pages.chain.x {
            level = level + hint;
        }
    }

    for (; level < inti_pages.chain.x; level = level + 1u) {
        let extent = base * exp2(f32(level));
        // Metres per texel at this level — why the offset is computed inside the walk.
        let texel_world = extent / f32(side * page_texels);
        // 🔴 Capped: the step is texels, 0.1 mm at level 0 and 5.12 m at 16, so uncapped it reaches
        // 9.2 m and walks receivers out of their shadow. ⚠️ `bias.z` 0 means no cap.
        var offset = texel_world * inti_pages.bias.x;
        if inti_pages.bias.z > 0.0 {
            offset = min(offset, inti_pages.bias.z);
        }
        let sampled = world_position
            + normal * offset
            + to_light * inti_pages.bias.y;

        // The plane is ABSOLUTE and the grid is snapped, so a texel's
        // footprint does not slide with the camera. See `sun_centre`.
        let centre = sun_centre(inti_pages.eye.xyz, basis, base, side, level);
        let along = dot(sampled - inti_pages.eye.xyz, basis[2])
            + sun_drift(inti_pages.eye.xyz, basis, base, side, level);
        // Reversed-Z along the sun's axis, matching `page_depth.wgsl`.
        // Nothing is added to it: the offset above already moved the
        // point towards the light, which is the depth half of the bias.
        let receiver = 1.0 - (along + span) / (2.0 * span);

        // The same absolute-world key the marking wrote. See `sun_cell`.
        let cell = sun_cell(sampled, inti_pages.eye.xyz, basis, base, side, level);
        // 🔴 The view is the key's high part: two viewports are two clipmaps.
        let page = inti_pages.views.x * inti_pages.views.y
            + inti_pages.space.w * inti_pages.space.x
            + level * side * side
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if slot == PAGE_MISS {
            continue;
        }

        // Where the point sits inside its own page, in texels.
        let rect = sun_page_rect(level, cell, inti_pages.eye.xyz, basis, base, side);
        let within = (sun_plane(sampled, basis) - rect.xy) / rect.z + vec2<f32>(0.5);
        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let origin = vec2<f32>(place.xy);
        let layer = i32(place.z);
        let texel = within * f32(page_texels);

        // 🔴 The receiver-plane gradient, in the texels of THIS level. `bias.w` clamps it, and 0
        // disables the term entirely — which is the A/B that says whether it is doing anything.
        let slope = receiver_slope(normal, basis, texel_world, span, inti_pages.bias.w);

        // Bilinear PCF, clamped inside the page — see
        // `inti_page_filter` for both halves of that sentence.
        return inti_page_filter(
            origin,
            layer,
            texel,
            receiver,
            slope,
            page_texels,
            cell,
            level,
            side,
        );
    }
    // No page anywhere in the chain. Lit, not shadowed: a point nobody marked is a point the frame
    // never looked at, and guessing dark there would put shadow where no data exists.
    return 1.0;
}
/// A local light's shadow from the page pool, so lamps past the cube budget still shadow. Walks
/// finest first and never lands coarser than marked; a spot writes the one face `mark_local`
/// assigns.
fn inti_local_page_shadow(
    light: u32,
    is_spot: bool,
    light_position: vec3<f32>,
    // The spot's axis; unread for a point. A spot's one face is aligned with it — see `spot_local`.
    light_direction: vec3<f32>,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
) -> f32 {
    let side0 = inti_pages.space.z;
    let page_texels = inti_pages.pool.w;
    let stride = inti_pages.space.x;
    let face_pages = inti_pages.space.y;
    // The chain stops where a whole level is one page. Mirrors `PageConfig::levels`.
    let levels = u32(log2(f32(max(side0, 1u)))) + 1u;
    let view_base = inti_pages.views.x * inti_pages.views.y;

    // 🔴 Starts at the floor, not at zero. The marking cannot pick a
    // level below it, so the levels under it hold no pages for anybody —
    // walking them is three table lookups a pixel that can only miss.
    for (var level = local_level_floor(side0 * page_texels); level < levels; level = level + 1u) {
        let side = level_side_of(level, side0);
        let raw = world_position - light_position;
        let distance = max(length(raw), PAGE_NEAR);
        // A 90° face covers `2 * distance` across `side * page_texels` texels — the identity
        // `page_level` inverts.
        let texel_world = 2.0 * distance / f32(side * page_texels);
        // 🔴 The point pair: `INTI_POINT_DEPTH_BIAS` is 4× the sun's. Borrowing the sun's printed
        // the self-shadow square under lamps.
        let sampled = world_position
            + normal * (texel_world * INTI_POINT_NORMAL_BIAS)
            + to_light * INTI_POINT_DEPTH_BIAS;

        var offset = sampled - light_position;
        if is_spot {
            offset = spot_local(light_direction, offset);
        }
        let hit = cube_face(offset);
        let face = select(u32(hit.w), 0u, is_spot);
        let cell = vec2<u32>(
            clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side)
        );
        // 🔴 The VIEW is the high part of the key, the same as the sun's: two viewports are two
        // page sets and a lookup without it finds whichever camera marked last.
        let page = view_base
            + light * stride
            + face * face_pages
            + local_level_base(level, side0, page_texels)
            + cell.y * side
            + cell.x;
        let slot = inti_page_lookup(page);
        if slot == PAGE_MISS {
            continue;
        }

        // Major-axis depth, as `page_depth.wgsl` stores `PAGE_NEAR / major`; radial distance drifts
        // toward face corners by up to 1.73×.
        let major = max(max(abs(offset.x), abs(offset.y)), abs(offset.z));
        let receiver = clamp(PAGE_NEAR / max(major, PAGE_NEAR), 0.0, 1.0);

        // Where the point sits inside its own cell, in texels.
        let step = 1.0 / f32(side);
        let low = vec2<f32>(cell) * step;
        let within = (hit.xy - low) / step;
        let place = page_place(slot, inti_pages.views.z, inti_pages.pool.z, page_texels);
        let origin = vec2<f32>(place.xy);
        let layer = i32(place.z);
        let texel = within * f32(page_texels);

        // No slope yet, deliberately: a perspective page needs a different derivation, and zero is
        // the old behaviour.
        let slope = vec2<f32>(0.0);

        // Bilinear PCF clamped in the page (see `inti_page_filter`); lamps clamp because an edge
        // crosses a face, not a grid step.
        return inti_page_filter(
            origin,
            layer,
            texel,
            receiver,
            slope,
            page_texels,
            vec2<u32>(0u),
            PAGE_UNLISTED,
            0u,
        );
    }
    // No page anywhere in the chain: lit, for the same reason the sun's
    // reader is. A point nobody marked is a point the frame never looked
    // at, and guessing dark there puts shadow where no data exists.
    return 1.0;
}

/// Sun occlusion at `world_position`, 1 = lit. Near a cascade split both cascades are sampled and
/// mixed, or the handover draws a line on the ground.
fn inti_shadow(
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    view_depth: f32,
    n_dot_l: f32,
) -> f32 {
    if (inti.shadows_enabled == 0u) {
        return 1.0;
    }
    // 🔴 Pages replace cascades rather than blending: two techniques disagree at their boundaries
    // and draw a seam.
    if (inti_pages.sun.w > 0.5) {
        return inti_page_shadow(world_position, normal, to_light, n_dot_l);
    }
    let picked = inti_pick_cascade(view_depth);
    let index = u32(picked.x);
    if (index >= 4u) {
        return 1.0;
    }

    var lit = inti_sample_cascade(index, world_position, normal, to_light, n_dot_l);
    if (picked.y <= 0.0) {
        return lit;
    }

    // Inside the overlap band. The last cascade has no successor, so it fades to lit instead — a
    // gradient into "no shadow data" rather than an edge at the end of the world.
    if (index == 3u) {
        return mix(lit, 1.0, picked.y);
    }
    let next = inti_sample_cascade(index + 1u, world_position, normal, to_light, n_dot_l);
    return mix(lit, next, picked.y);
}

// Everything about a shaded point independent of the light, built once per pixel — a struct so
