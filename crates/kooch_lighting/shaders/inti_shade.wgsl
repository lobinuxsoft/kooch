// debug views run the same per-light maths. WGSL inlines it.
struct IntiSurface {
    world_position: vec3<f32>,
    n: vec3<f32>,
    // Towards the camera.
    v: vec3<f32>,
    // The mirror direction; only the representative point (#776) reads it, but it is per pixel.
    r: vec3<f32>,
    n_dot_v: f32,
    diffuse_color: vec3<f32>,
    f0: vec3<f32>,
    perceptual: f32,
    // Linear roughness, `perceptual²`.
    a: f32,
    f_ab: vec2<f32>,
    // Distance along the view axis, which picks a cascade: radial distance makes boundaries spheres
    // that sweep across the frame.
    view_depth: f32,
    // #804 — the instance's bits. Bit 0 is "receives shadows"; when it
    // is clear, `inti_light_contribution` skips the shadow fetch
    // outright rather than fetching and multiplying by one.
    flags: u32,
    // #1220 — the instance's layers. A light whose own mask shares no bit with it is skipped before
    // anything is sampled.
    layers: u32,
}

// `base_color` is linear albedo (sRGB textures are decoded by the sampler; `Material::base_color`
// is documented linear). `metallic` and `roughness` are the usual perceptual [0,1] scalars.
fn inti_surface(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    flags: u32,
    layers: u32,
) -> IntiSurface {
    let v = normalize(inti.camera_position - world_position);
    let n_dot_v = max(dot(n, v), 1e-4);
    let perceptual = clamp(roughness, INTI_MIN_PERCEPTUAL_ROUGHNESS, 1.0);

    var surf: IntiSurface;
    surf.world_position = world_position;
    surf.n = n;
    surf.v = v;
    surf.r = reflect(-v, n);
    surf.n_dot_v = n_dot_v;
    // Metals have no diffuse and take F0 from their albedo;
    // dielectrics reflect a flat 4% and keep all of theirs.
    surf.diffuse_color = base_color * (1.0 - metallic);
    surf.f0 = mix(vec3<f32>(0.04), base_color, metallic);
    surf.perceptual = perceptual;
    surf.a = perceptual * perceptual;
    surf.f_ab = inti_f_ab(perceptual, n_dot_v);
    surf.view_depth = dot(world_position - inti.camera_position, inti.camera_forward);
    surf.flags = flags;
    surf.layers = layers;
    return surf;
}

// Karis 2013's representative point (s2013_pbs_epic_notes_v2.pdf p14–16): the sphere point nearest
// the mirror ray in `xyz`, the widened linear roughness in `w`.
fn inti_representative_point(
    r: vec3<f32>,
    a: f32,
    offset: vec3<f32>,
    radius: f32,
    distance: f32,
) -> vec4<f32> {
    // 🔴 A fix, not a divide-by-zero guard (bevy#13318): surfaces inside the light's sphere show a
    // discontinuity without it.
    let lt_f_dot_r = max(0.0001, dot(offset, r));
    let center_to_ray = lt_f_dot_r * r - offset;
    let closest = offset + center_to_ray * saturate(
        radius * inverseSqrt(dot(center_to_ray, center_to_ray)));
    // Karis p14. The 2 is hand-tuned against reference renders, not derived.
    let a_prime = saturate(a + radius / (2.0 * max(distance, 1e-4)));
    return vec4<f32>(closest * inverseSqrt(dot(closest, closest)), a_prime);
}

// Bevy's amendment: lerp original and widened roughness by roughness, or smooth materials read
// rough and dim. LTC (#779) is the real fix.
fn inti_specular_fix_remap(a: f32) -> f32 {
    let inv_a_sq = (1.0 - a) * (1.0 - a);
    return 1.0 - inv_a_sq * inv_a_sq;
}

// One light's answer, with enough of the question left attached for the
// caller to decide which light mattered most (#845).
struct IntiLit {
    // What this light adds to the surface point.
    radiance: vec3<f32>,
    // The ceiling this light had on this point: irradiance faced head-on, times the cosine. The
    // same number the range cut and `specular_floor` already read — a light's weight in one scalar.
    reach: f32,
    // Unit vector toward it, which is what a contact march needs.
    to_light: vec3<f32>,
}

// What one light adds to one surface point, shadows included. Zero when
// the surface faces away from it.
fn inti_light_contribution(
    surf: IntiSurface,
    light: IntiLight,
    // 🔴 The light's buffer index, its identity in page keys — passed in, since `IntiLight`'s pads
    // are load-bearing.
    index: u32,
    frag_coord: vec2<f32>,
) -> vec3<f32> {
    return inti_light_lit(surf, light, index, frag_coord, true).radiance;
}

// The same, with the contact march optional and the light's weight reported. 🔴 `march == false`
// means not here: the caller marches once, for the highest `reach`.
fn inti_light_lit(
    surf: IntiSurface,
    light: IntiLight,
    index: u32,
    frag_coord: vec2<f32>,
    march: bool,
) -> IntiLit {
    // Before anything is sampled (#1220): a light that does not light this surface's layers costs
    // one AND, not a shadow fetch and a BRDF.
    if ((light.layers & surf.layers) == 0u) {
        return IntiLit(vec3<f32>(0.0), 0.0, vec3<f32>(0.0, 1.0, 0.0));
    }
    let s = inti_sample_light(light, surf.world_position);
    let n_dot_l = dot(surf.n, s.to_light);
    let nothing = IntiLit(vec3<f32>(0.0), 0.0, s.to_light);
    if (n_dot_l <= 0.0) {
        return nothing;
    }

    // The most this light can put here. 🔴 Zero is exact past `range` (#835), so return now instead
    // of multiplying both BRDF layers, the cube and the march by it — the froxel conservatively
    // admits ~26 of ~40 such lights (#820).
    let reach = max(max(s.irradiance.x, s.irradiance.y), s.irradiance.z) * n_dot_l;
    if (reach <= 0.0) {
        return nothing;
    }

    // The diffuse layer always answers to the light's centre.
    let h = normalize(s.to_light + surf.v);
    let l_dot_h = saturate(dot(s.to_light, h));
    let diffuse_term = inti_fd_burley(surf.perceptual, surf.n_dot_v, n_dot_l, l_dot_h);

    // The expensive half — GGX, Smith, Schlick, multiscatter, representative point — per light per
    // pixel with ~15 lights (#820). Under `specular_floor` (#821) a light pays diffuse only; 0.0
    // always takes it.
    var specular = vec3<f32>(0.0);
    var n_dot_l_spec = n_dot_l;
    // ⚠️ Fresnel at normal incidence stays when specular is skipped: `f` weights diffuse, and
    // dropping it would brighten the surface.
    var f = surf.f0;
    // `reach` is the same value the range cut above already computed —
    // one light's ceiling on this surface, read twice for two different
    // questions. This one asks whether the highlight is worth its cost.
    if (reach >= inti.specular_floor) {
        var l_spec = s.to_light;
        var a_spec = surf.a;
        var l_dot_h_spec = l_dot_h;
        var h_spec = h;
        var spec_intensity = 1.0;
        var solid_angle = 0.0;
        // Directional lights are excluded by kind: no position, no distance to correct; the sun's
        // size is a shadow problem (#477).
        if (light.radius > 0.0 && light.kind != INTI_KIND_DIRECTIONAL) {
            let rep = inti_representative_point(
                surf.r, surf.a, s.offset, light.radius, s.distance);
            l_spec = rep.xyz;
            h_spec = normalize(l_spec + surf.v);
            n_dot_l_spec = saturate(dot(surf.n, l_spec));
            l_dot_h_spec = saturate(dot(l_spec, h_spec));
            // Spreading energy must not add any. Uses the raw widened roughness while the BRDF gets
            // the remapped one, as Bevy.
            let normalization = surf.a / max(rep.w, 1e-4);
            spec_intensity = normalization * normalization;
            a_spec = mix(surf.a, rep.w, inti_specular_fix_remap(surf.a));
            // Sphere visibility: at a grazing angle part of the sphere has
            // sunk below the horizon and cannot light this point at all.
            solid_angle = light.radius * light.radius
                / max(s.distance * s.distance, 1e-8);
        }

        let n_dot_h = saturate(dot(surf.n, h_spec));
        let d = inti_d_ggx(a_spec, n_dot_h);
        let vis = inti_v_smith_correlated(a_spec, surf.n_dot_v, n_dot_l_spec);
        f = inti_fresnel(surf.f0, l_dot_h_spec);
        specular = inti_specular_multiscatter(
            d * vis * f * spec_intensity, surf.f0, surf.f_ab);
        if (solid_angle > 0.0) {
            specular *= saturate(n_dot_l_spec / max(n_dot_l_spec + solid_angle, 1e-4));
        }
    }

    // Diffuse gets what specular did not reflect; Bevy's forward path skips this, its path tracer
    // does not.
    let diffuse = (vec3<f32>(1.0) - f) * surf.diffuse_color * diffuse_term;

    // Each light kind reads its own map: cascades (#476), spot layer (#777), point cube (#778). A
    // non-receiving surface fetches nothing (#804).
    var shadow = 1.0;
    if ((surf.flags & INTI_SURFACE_RECEIVES_SHADOWS) != 0u) {
        if (light.kind == INTI_KIND_DIRECTIONAL) {
            shadow = inti_shadow(surf.world_position, surf.n, s.to_light, surf.view_depth, n_dot_l);
        } else if (light.kind == INTI_KIND_POINT) {
            // 🔴 Pages are not gated on `shadow_slot`, a cube index capped at 32; page-backed lamps
            // are keyed by light index.
            if (inti_pages.sun.w > 0.5) {
                shadow = inti_local_page_shadow(
                    index, false, light.position, light.direction,
                    surf.world_position, surf.n, s.to_light);
            } else if (light.shadow_slot != INTI_NO_SHADOW_SLOT) {
                // Six faces of one cube (#778).
                shadow = inti_point_shadow(
                    light.shadow_slot, surf.world_position, surf.n, s.to_light, light.position);
            }
        } else if (light.kind == INTI_KIND_SPOT) {
            if (inti_pages.sun.w > 0.5) {
                // One face, aligned with the spot's own axis.
                shadow = inti_local_page_shadow(
                    index, true, light.position, light.direction,
                    surf.world_position, surf.n, s.to_light);
            } else if (light.shadow_slot != INTI_NO_SHADOW_SLOT) {
                // A spot casts into the cascade array (#777); axial distance as Bevy, since radial
                // would widen the bias at the cone's edge.
                let axial = dot(light.direction, surf.world_position - light.position);
                shadow = inti_spot_shadow(
                    light.shadow_slot, surf.world_position, surf.n, s.to_light, n_dot_l, axial);
            }
        }

        // Contact shadows (#735) for any light kind, skipped where the map already shadows: two
        // occlusions of one occluder darken twice.
        if (march && (light.flags & INTI_LIGHT_CONTACT_SHADOWS) != 0u && shadow > 0.0) {
            shadow *= inti_contact_shadow(
                surf.world_position, surf.n, surf.v, s.to_light, frag_coord);
        }
    }

    // 🔴 Cosine per layer, as Bevy: specular has its own N·L, and factoring it out undoes half of
    // #776 invisibly until a radius is authored.
    let radiance = (diffuse * n_dot_l + specular * n_dot_l_spec) * s.irradiance * shadow;
    // Only a light whose own map does not block it may win the one march.
    let marchable = shadow > 0.0 && (light.flags & INTI_LIGHT_CONTACT_SHADOWS) != 0u;
    return IntiLit(radiance, select(0.0, reach, marchable), s.to_light);
}

/// A point light's shadow, one cube (#778). Depth is the largest axis component, not the length —
/// faces meet at 45°, and length scales the bias by up to √3 at corners.
fn inti_point_shadow(
    slot: u32,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    light_position: vec3<f32>,
) -> f32 {
    if (slot >= inti.point_shadow_count) {
        return 1.0;
    }
    let record = inti.point_shadows[slot];

    let surface_to_light = light_position - world_position;
    let abs_to_light = abs(surface_to_light);
    let distance_to_light =
        max(abs_to_light.x, max(abs_to_light.y, abs_to_light.z));

    // The same two world-space offsets the cascades and the spots use, with the texel size resolved
    // to metres by this fragment's own distance — the record carries an angle, not a length.
    let texel_world = record.texel_world_size * distance_to_light;
    let offset_position = world_position
        + normal * (texel_world * INTI_POINT_NORMAL_BIAS)
        + to_light * INTI_POINT_DEPTH_BIAS;

    let frag_ls = offset_position - light_position;
    let abs_ls = abs(frag_ls);
    let major = max(abs_ls.x, max(abs_ls.y, abs_ls.z));
    // The whole depth reconstruction under infinite reverse-Z; nearer is greater, as `Greater`
    // expects.
    let depth = record.near / max(major, 1e-4);

    // 🔴 Cube maps are left-handed and the engine is not: faces are stored Z-swapped
    // (`FACE_DIRECTIONS`) and the direction mirrored here. Fixing one half flips shadows.
    let dir = frag_ls * vec3<f32>(1.0, 1.0, -1.0);
    return inti_filter_cube(dir, depth, slot, texel_world);
}

// Branchless orthonormal basis (Duff et al. 2017, Bevy's `orthonormalize`): cube filter taps move
// across the sampling direction's tangent plane. `z_basis` must be unit.
fn inti_orthonormalize(z_basis: vec3<f32>) -> mat3x3<f32> {
    let sign = select(-1.0, 1.0, z_basis.z >= 0.0);
    let a = -1.0 / (sign + z_basis.z);
    let b = z_basis.x * z_basis.y * a;
    return mat3x3<f32>(
        vec3<f32>(1.0 + sign * z_basis.x * z_basis.x * a, sign * b, -sign * z_basis.x),
        vec3<f32>(b, sign + z_basis.y * z_basis.y * a, -z_basis.y),
        z_basis,
    );
}

// Cube filter width in texels — Bevy's 0.003 in direction units is about one texel at 1024², but
// units of texels survive face-size changes. Wider on purpose: 512² faces, and a lamp's shadow
// should be soft.
const INTI_POINT_FILTER_TEXELS: f32 = 2.0;

// Eight D3D MSAA positions with Gaussian weights summing to 1. ⚠️ Not Castano: its bilinear
// nine-from-four trick does not exist for cubemaps.
fn inti_filter_cube(
    dir: vec3<f32>,
    depth: f32,
    slot: u32,
    // 🔴 Already metres at this fragment's distance; multiplying by distance again grew the radius
    // with its square.
    texel_world: f32,
) -> f32 {
    let positions = array<vec2<f32>, 8>(
        vec2<f32>(0.125, -0.375),
        vec2<f32>(-0.125, 0.375),
        vec2<f32>(0.625, 0.125),
        vec2<f32>(-0.375, -0.625),
        vec2<f32>(-0.625, 0.625),
        vec2<f32>(-0.875, -0.125),
        vec2<f32>(0.375, 0.875),
        vec2<f32>(0.875, -0.875),
    );
    let coeffs = array<f32, 8>(
        0.157112,
        0.157112,
        0.138651,
        0.130251,
        0.114946,
        0.114946,
        0.107982,
        0.079001,
    );

    // The tangent plane scaled so one unit is one texel at this distance, matching the direction
    // vector's length.
    let basis = inti_orthonormalize(normalize(dir))
        * (texel_world * INTI_POINT_FILTER_TEXELS);

    var sum = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        let offset = positions[i].x * basis[0] + positions[i].y * basis[1];
        sum += coeffs[i] * textureSampleCompareLevel(
            inti_point_cubes, inti_shadow_sampler, dir + offset, i32(slot), depth);
    }
    return sum;
}

/// A spot light's shadow (#777): one map, no splits, so the cascade path on the spot's record.
fn inti_spot_shadow(
    slot: u32,
    world_position: vec3<f32>,
    normal: vec3<f32>,
    to_light: vec3<f32>,
    n_dot_l: f32,
    distance_to_light: f32,
) -> f32 {
    if (slot >= inti.spot_shadow_count) {
        return 1.0;
    }
    // Normal bias × distance, as Bevy: a perspective texel grows with distance, so no per-scene
    // tuning.
    return inti_sample_cascade_record(
        inti.spot_shadows[slot], world_position, normal, to_light, n_dot_l,
        max(distance_to_light, 0.0));
}

// The running sum, plus the strongest light seen so far.
struct IntiAccum {
    radiance: vec3<f32>,
    // That light's own radiance, so the march can be applied to it and to nothing else.
    brightest: vec3<f32>,
    reach: f32,
    to_light: vec3<f32>,
}

fn inti_accumulate(acc: IntiAccum, lit: IntiLit) -> IntiAccum {
    var out = acc;
    out.radiance += lit.radiance;
    // Strictly greater, so the first of two equals wins and the choice
    // does not depend on which order the froxel happened to list them.
    if (lit.reach > out.reach) {
        out.brightest = lit.radiance;
        out.reach = lit.reach;
        out.to_light = lit.to_light;
    }
    return out;
}

fn inti_merge(a: IntiAccum, b: IntiAccum) -> IntiAccum {
    var out = a;
    out.radiance += b.radiance;
    if (b.reach > out.reach) {
        out.brightest = b.brightest;
        out.reach = b.reach;
        out.to_light = b.to_light;
    }
    return out;
}

/// The whole model for one surface point, as linear HDR radiance; exposure and transfer are
/// `inti_tonemap`'s, so other passes can read the raw value.
fn inti_shade(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    frag_coord: vec2<f32>,
    // #804 — the instance's bits, straight off `VertexOutput.flags`.
    flags: u32,
    // #1220 — and its layers, off the same instance.
    layers: u32,
) -> vec3<f32> {
    let surf = inti_surface(world_position, n, base_color, metallic, roughness, flags, layers);

    // 🔴 One march per pixel, not per light (#845): 1.7 ms per step on the OneXFly × ~14 lights was
    // the whole frame. The loop remembers the brightest light and marches for it alone.
    var acc = IntiAccum(vec3<f32>(0.0), vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    let dominant = inti_contact_dominant_only();
    if (inti.clustered == 0u) {
        // No grid this frame: every light for every pixel, as before #780 and in headless paths.
        for (var i = 0u; i < inti.light_count; i = i + 1u) {
            acc = inti_accumulate(acc, inti_light_lit(
                surf, inti_lights[i], i, frag_coord, !dominant));
        }
    } else {
        // Directional lights are not in the grid: they reach every cell, so a cell listing them
        // would say nothing. They are the light buffer's leading entries — see `ExtractedLights`.
        for (var i = 0u; i < inti.directional_count; i = i + 1u) {
            acc = inti_accumulate(acc, inti_light_lit(
                surf, inti_lights[i], i, frag_coord, !dominant));
        }
        acc = inti_merge(acc, inti_clustered_lights(surf, world_position, frag_coord, dominant));
    }

    var radiance = acc.radiance;
    if (dominant && acc.reach > 0.0) {
        // Subtracting what the march removes, rather than re-adding a
        // shaded copy: the winner's radiance is already inside the sum.
        let shadow = inti_contact_shadow(
            surf.world_position, surf.n, surf.v, acc.to_light, frag_coord);
        radiance -= acc.brightest * (1.0 - shadow);
    }

    radiance += inti_ambient(n, surf.diffuse_color, surf.f0, surf.f_ab);
    return radiance;
}


// This fragment's froxel lights only (#780): bounds from the cell, so a distant lamp costs nothing.
// Points and spots are two consecutive ranges, no type test.
fn inti_clustered_lights(
    surf: IntiSurface,
    world_position: vec3<f32>,
    frag_coord: vec2<f32>,
    dominant: bool,
) -> IntiAccum {
    let cell = inti_clusters[inti_cluster_of(world_position, frag_coord)];
    let points_end = cell.offset + cell.point_count;
    // Reflection probes, irradiance volumes and decals follow the spots in the same record. Nothing
    // reads them yet; when something does, it reads its own range and this loop does not change.
    var spots_end = points_end + cell.spot_count;
    // `KOOCH_LIGHT_LIMIT`, applied here so both shading paths inherit it.
    if (inti.light_limit != 0u) {
        spots_end = min(spots_end, cell.offset + inti.light_limit);
    }

    // #839: taps are steps × lights; this caps the lights that march (0 = no cap). Past it a light
    // still shades.
    let march_cap = inti_contact_max_lights();
    var marched = 0u;

    var acc = IntiAccum(vec3<f32>(0.0), vec3<f32>(0.0), 0.0, vec3<f32>(0.0));
    for (var i = cell.offset; i < spots_end; i = i + 1u) {
        // 🔴 Clamped to the list's length: an overflowed frame points past it, and under-lit beats
        // an undefined read.
        if (i >= inti.cluster_capacity) {
            break;
        }
        let index = inti_cluster_indices[i];
        let march = !dominant && (march_cap == 0u || marched < march_cap);
        if march && (inti_lights[index].flags & INTI_LIGHT_CONTACT_SHADOWS) != 0u {
            marched = marched + 1u;
        }
        acc = inti_accumulate(acc, inti_light_lit(
            surf, inti_lights[index], index, frag_coord, march));
    }
    return acc;
}

// The fragment's cell in grid coordinates, split out for #824: a compute tile reduces cells to
// per-axis min/max, which a linear index cannot.
fn inti_cluster_cell(world_position: vec3<f32>, frag_coord: vec2<f32>) -> vec3<u32> {
    let view_z = dot(inti.view_z_row, vec4<f32>(world_position, 1.0));
    let xy = vec2<u32>(floor(frag_coord * inti.cluster_factors.xy));
    // Mirrors `cluster_z_slice` and `ClusterGrid::z_slice`; disagreeing reads a cell the grid never
    // wrote.
    let slice = log(-view_z) * inti.cluster_factors.z - inti.cluster_factors.w + 1.0;
    let z = min(u32(max(slice, 0.0)), inti.cluster_dimensions.z - 1u);
    return clamp(
        vec3<u32>(xy, z),
        vec3<u32>(0u),
        inti.cluster_dimensions.xyz - vec3<u32>(1u));
}

// That cell's index into `inti_clusters`.
fn inti_cluster_index(cell: vec3<u32>) -> u32 {
    return min(
        (cell.y * inti.cluster_dimensions.x + cell.x) * inti.cluster_dimensions.z + cell.z,
        inti.cluster_dimensions.w - 1u);
}

// Which cell of the grid a fragment is in.
fn inti_cluster_of(world_position: vec3<f32>, frag_coord: vec2<f32>) -> u32 {
    return inti_cluster_index(inti_cluster_cell(world_position, frag_coord));
}

// The operator is in `inti_tonemap.wgsl` (`INTI_TONEMAP`), shared with the standalone tonemap pass
// (#732), which has no `inti` binding.
fn inti_tonemap(radiance: vec3<f32>) -> vec3<f32> {
    return inti_tonemap_with(radiance, inti.exposure);
}
