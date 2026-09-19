// How many froxels of this view hold visible surface — the population of the occupancy bitmap
// `mark_pixel` fills.
@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn count_froxels(@builtin(local_invocation_index) lane: u32) {
    let base = rank_base() + RANK_OCCUPANCY;
    var found = 0u;
    // 64 lanes over `OCCUPANCY_WORDS`, strided so the loop is the same length in every lane.
    for (var w = lane; w < OCCUPANCY_WORDS; w = w + MARK_GROUP * MARK_GROUP) {
        found = found + countOneBits(atomicLoad(&rank_state[base + w]));
    }
    if found != 0u {
        atomicAdd(&counters[9], found);
    }
}

// Marking per (froxel, light) instead of per (pixel, light) — #952.

/// Cells a single (froxel, light) pair may mark on one face before it
/// gives up a level. A froxel close to a light spans a wide angle, and
/// an unbounded rect there would spend the pool on one pair.
const FROXEL_RECT_MAX: u32 = 4u;

/// The world-space corners of a froxel, from its grid coordinate.
fn froxel_corner(bounds: ClusterAabb, corner: u32, world_from_view: mat4x4<f32>) -> vec3<f32> {
    let pick = vec3<f32>(
        f32(corner & 1u),
        f32((corner >> 1u) & 1u),
        f32((corner >> 2u) & 1u),
    );
    let view_pos = mix(bounds.min, bounds.max, pick);
    let world = world_from_view * vec4<f32>(view_pos, 1.0);
    return world.xyz / world.w;
}

@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn mark_froxels(@builtin(global_invocation_id) gid: vec3<u32>) {
    let froxel = gid.x;
    if froxel >= view.dimensions.w || froxel >= OCCUPANCY_MAX {
        return;
    }
    // Occupied only. The bitmap is what keeps the depth buffer's free occlusion culling: a froxel
    // of empty air, or one behind a wall, has no bit and marks nothing.
    let bits = atomicLoad(&rank_state[rank_base() + RANK_OCCUPANCY + froxel / 32u]);
    if (bits & (1u << (froxel % 32u))) == 0u {
        return;
    }
    let record = cells[froxel];
    let count = record.point_count + record.spot_count;
    if count == 0u {
        return;
    }
    // The worst overlap in the frame. See `MarkCounts::peak_lights`.
    atomicMax(&counters[16], count);

    // The grid coordinate back out of the flat index; mirrors `cluster_index`.
    let dz = max(view.dimensions.z, 1u);
    let dx = max(view.dimensions.x, 1u);
    let cell = vec3<u32>((froxel / dz) % dx, (froxel / dz) / dx, froxel % dz);
    var bounds = cluster_cell_bounds(view, cell);
    // 🔴 The froxel's box narrowed to the slab its SURFACE occupies — Olsson's explicit bounds
    // against its implicit ones. Without this the pass marks pages for the empty depth either side
    // of a thin sheet, and the pool pays for every one of them.
    let slab = rank_base() + RANK_DEPTH + froxel * 2u;
    let far_bits = atomicLoad(&rank_state[slab + 1u]);
    if far_bits != 0u {
        let near_bits = ~atomicLoad(&rank_state[slab]);
        // View space looks down -Z, so the nearer surface is the LARGER z. Clamped INTO the
        // froxel's own box, never outside it: the slab is what the samples reached and the box is
        // what the addressing says this cell covers.
        let a_z = -bitcast<f32>(near_bits);
        let b_z = -bitcast<f32>(far_bits);
        bounds.min.z = clamp(min(a_z, b_z), bounds.min.z, bounds.max.z);
        bounds.max.z = clamp(max(a_z, b_z), bounds.min.z, bounds.max.z);
    }
    let world_from_view = pages.world_from_clip * view.clip_from_view;

    var corners: array<vec3<f32>, 8>;
    for (var c = 0u; c < 8u; c = c + 1u) {
        corners[c] = froxel_corner(bounds, c, world_from_view);
    }
    // What a screen pixel covers at this froxel's FAR face — the same quantity `mark_pixel`
    // computes per sample, evaluated once for the whole cell and at its coarsest end.
    let focal = view.clip_from_view[1][1];
    var wanted = 0.0;
    if abs(focal) > 1e-9 {
        wanted = 2.0 * abs(bounds.min.z) / (focal * max(view.viewport.y, 1.0));
    }
    let bias = atomicLoad(&rank_state[rank_base() + RANK_BIAS]);
    wanted = wanted * pages.density.x * exp2(f32(bias & 0xffu));

    var pairs = 0u;
    var culled = 0u;
    var distant = 0u;
    for (var i = 0u; i < count; i = i + 1u) {
        let slot = record.offset + i;
        if slot >= arrayLength(&indices) {
            break;
        }
        let light = indices[slot];
        if light >= pages.strides.w {
            continue;
        }
        pairs = pairs + 1u;
        // 🔴 COUNTED, not merely skipped. A lamp under the projected-size gate (#944) casting
        // nothing is indistinguishable from a lamp that was never reached, and the panel reads this
        // number to tell "the gate is working" from "the light is missing".
        if light_gated(light) {
            culled = culled + 1u;
            continue;
        }
        if light_distant(light) {
            distant = distant + 1u;
        }
        mark_froxel_light(light, corners, wanted);
    }
    // One thread per FROXEL, thousands rather than millions, so these go straight to the counters —
    // the workgroup reduction `mark_pixel` needs buys nothing at this width. See `mark_flush`.
    if pairs != 0u {
        atomicAdd(&counters[2], pairs);
    }
    if culled != 0u {
        atomicAdd(&counters[6], culled);
    }
    if distant != 0u {
        atomicAdd(&counters[24], distant);
    }
}

/// Marks every page one froxel needs from one light.
fn mark_froxel_light(light: u32, corners: array<vec3<f32>, 8>, wanted: f32) {
    var corner_pages: array<vec2<u32>, 8>;
    var level = 0u;
    var reach = 0.0;
    let record = lights[light];
    // Pass one: the coarsest level any corner asks for, and the furthest
    // corner, which is this froxel's receiver bound (#940).
    for (var c = 0u; c < 8u; c = c + 1u) {
        let page = local_page_for(light, corners[c], wanted);
        corner_pages[c] = page;
        level = max(level, page.y);
        reach = max(reach, length(corners[c] - record.position));
    }
    // Pass two: every corner re-read at that one level, so the cells are
    // comparable and the rect between them means something.
    var seen_faces = 0u;
    for (var c = 0u; c < 8u; c = c + 1u) {
        let face_cell = local_cell_at(light, corners[c], level);
        let face = face_cell.z;
        if (seen_faces & (1u << face)) != 0u {
            continue;
        }
        seen_faces = seen_faces | (1u << face);
        // The rect this face spans, over the corners that landed on it.
        var lo = face_cell.xy;
        var hi = face_cell.xy;
        for (var d = c + 1u; d < 8u; d = d + 1u) {
            let other = local_cell_at(light, corners[d], level);
            if other.z != face {
                continue;
            }
            lo = min(lo, other.xy);
            hi = max(hi, other.xy);
        }
        mark_face_rect(light, face, level, lo, hi, reach);
    }
}

/// A point's (cell.xy, face) on a light's chain at a GIVEN level.
fn local_cell_at(light: u32, world: vec3<f32>, level: u32) -> vec3<u32> {
    let record = lights[light];
    var offset = world - record.position;
    let spot = record.kind == PAGE_KIND_SPOT;
    if spot {
        offset = spot_local(record.direction, offset);
    }
    let hit = cube_face(offset);
    let face = select(u32(hit.w), 0u, spot);
    let side = level_side(level);
    let cell = vec2<u32>(clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side));
    return vec3<u32>(cell, face);
}

/// Marks the cells `lo..=hi` of one face, coarsening until the rect fits [`FROXEL_RECT_MAX`] on
/// both axes.
fn mark_face_rect(
    light: u32,
    face: u32,
    start_level: u32,
    lo_in: vec2<u32>,
    hi_in: vec2<u32>,
    reach: f32,
) {
    var level = start_level;
    var lo = lo_in;
    var hi = hi_in;
    // `level_side` halves with each level, so the cells do too.
    for (var guard = 0u; guard < 8u; guard = guard + 1u) {
        let span = hi - lo + vec2<u32>(1u);
        if (span.x <= FROXEL_RECT_MAX && span.y <= FROXEL_RECT_MAX)
            || level + 1u >= pages.chain.z {
            break;
        }
        level = level + 1u;
        lo = lo / 2u;
        hi = hi / 2u;
    }
    let side = level_side(level);
    let base = view_base() + light * pages.strides.z + face * pages.strides.y + level_base(level);
    let top = min(hi, vec2<u32>(max(side, 1u) - 1u));
    for (var y = lo.y; y <= top.y; y = y + 1u) {
        for (var x = lo.x; x <= top.x; x = x + 1u) {
            let index = base + y * side + x;
            if index >= pages.pool.x {
                atomicAdd(&counters[3], 1u);
                continue;
            }
            // rather than a sample: larger, so it rejects less, which is the safe direction for a
            // bound that culls casters. 🔴 `mark_bit`, not `page_touch`.
            if mark_bit(index, true) {
                atomicAdd(&rank_state[rank_base() + rank_local(level)], 1u);
                page_touch(index);
            }
        }
    }
}
