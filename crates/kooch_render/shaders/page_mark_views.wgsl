/// 🔴 The barriers live HERE and the work lives in `mark_pixel`, because `workgroupBarrier` in
/// non-uniform control flow is undefined and `mark_pixel` returns early three ways — off the edge
/// of the viewport, on sky, on a degenerate reconstruction.
@compute @workgroup_size(MARK_GROUP, MARK_GROUP, 1)
fn mark_main(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    if lane < 5u {
        atomicStore(&tally[lane], 0u);
    }
    workgroupBarrier();
    mark_pixel(id);
    workgroupBarrier();
    if lane == 0u {
        let samples = atomicLoad(&tally[TALLY_SAMPLES]);
        if samples != 0u {
            atomicAdd(&counters[1], samples);
        }
        let pairs = atomicLoad(&tally[TALLY_PAIRS]);
        if pairs != 0u {
            atomicAdd(&counters[2], pairs);
        }
        let culled = atomicLoad(&tally[TALLY_CULLED]);
        if culled != 0u {
            atomicAdd(&counters[6], culled);
        }
        let peak = atomicLoad(&tally[TALLY_PEAK]);
        if peak != 0u {
            atomicMax(&counters[16], peak);
        }
        let distant = atomicLoad(&tally[TALLY_DISTANT]);
        if distant != 0u {
            atomicAdd(&counters[24], distant);
        }
    }
}

fn mark_pixel(id: vec3<u32>) {
    let rate = max(pages.sampling.x, 1u);
    let pixel = id.xy * rate;
    let size = vec2<u32>(view.viewport.xy);
    if pixel.x >= size.x || pixel.y >= size.y {
        return;
    }

    // 🔴 Reversed-Z infinite (ADR 0002): the buffer clears to 0 and that is the FAR value, so a zero
    // is sky rather than a surface at the near plane. Marking it would put a page under every pixel
    // the scene does not cover, which is the whole failure this pass exists to avoid.
    let depth = textureLoad(depth_tex, vec2<i32>(pixel), 0);
    if depth <= 0.0 {
        return;
    }
    atomicAdd(&tally[TALLY_SAMPLES], 1u);

    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / view.viewport.xy;
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let hom = pages.world_from_clip * vec4<f32>(ndc, 1.0);
    if abs(hom.w) < 1e-9 {
        return;
    }
    let world = hom.xyz / hom.w;

    // World metres one screen pixel covers here, from the frustum rather than from the froxel: `2 *
    // d * tan(fov/2) / height`, with the focal length read off the projection. Mirrors
    // `CensusCamera::pixel_at`.
    let view_pos = view.view_from_world * vec4<f32>(world, 1.0);
    let focal = view.clip_from_view[1][1];
    var wanted = 0.0;
    if abs(focal) > 1e-9 {
        wanted = 2.0 * abs(view_pos.z) / (focal * max(view.viewport.y, 1.0));
    }
    // A sample is `rate` pixels wide when the pass runs coarse, and the
    // page it needs has to cover all of them.
    wanted = wanted * f32(rate) * pages.density.x;
    // The pressure bias (#943): what last frame's plan learned, applied
    // to what this frame asks for.
    let bias = atomicLoad(&rank_state[rank_base() + RANK_BIAS]);
    let local_wanted = wanted * exp2(f32(bias & 0xffu));

    if pages.sun.w > 0.5 {
        // One diagonal per thread, which one decided by the thread's own parity — Epic's
        // `PageDilationDither`, off `GroupIndex` there.
        let dither = vec2<f32>(
            select(-1.0, 1.0, (id.x & 1u) != 0u),
            select(-1.0, 1.0, (id.y & 1u) != 0u),
        );
        _ = mark_sun(pages.sampling.y, world, wanted * exp2(f32(bias >> 8u)), dither);
    }

    if view.dimensions.w == 0u {
        return;
    }
    let froxel = cluster_index(cluster_of_ndc(view, ndc, view_pos.z), view.dimensions);
    // This froxel holds visible surface. One `atomicOr` per pixel, and it replaces nothing yet —
    // see `RANK_OCCUPANCY` for what it is for and why the depth buffer's answer has to be kept.
    if froxel < OCCUPANCY_MAX {
        atomicOr(
            &rank_state[rank_base() + RANK_OCCUPANCY + froxel / 32u],
            1u << (froxel % 32u),
        );
        // The explicit bounds: how deep this froxel's SURFACE actually runs. Positive floats
        // bitcast to u32 keep their order, which is what lets an atomic hold a depth.
        let depth_bits = bitcast<u32>(abs(view_pos.z));
        let slab = rank_base() + RANK_DEPTH + froxel * 2u;
        atomicMax(&rank_state[slab], ~depth_bits);
        atomicMax(&rank_state[slab + 1u], depth_bits);
    }
    let record = cells[froxel];
    // Recorded on BOTH paths: the overlap is a property of the scene, not of how the marking walks
    // it, and the alert has to mean the same thing whichever is on.
    atomicMax(&tally[TALLY_PEAK], record.point_count + record.spot_count);
    // 🔴 The per-light walk belongs to `mark_froxels` when the cluster path is on (#952). Everything
    // above this line still runs: the sun is marked per pixel, and the occupancy bit that pass
    // reads was set above.
    if pages.density.z > 0.5 {
        return;
    }
    let start = record.offset;
    // Points and spots are the first two ranges, stored in that order,
    // and both need pages. Probes, volumes and decals do not.
    let count = record.point_count + record.spot_count;
    // 🔴 Counted in registers first. Even an LDS atomic is a shared address, and this loop body runs
    // once per light per pixel — the hottest place in the pass. One thread's whole cluster costs it
    // two workgroup atomics at the end instead of two per light.
    var pairs = 0u;
    var culled = 0u;
    var distant = 0u;
    for (var i = 0u; i < count; i = i + 1u) {
        let slot = start + i;
        if slot >= arrayLength(&indices) {
            break;
        }
        let light = indices[slot];
        if light >= pages.strides.w {
            continue;
        }
        pairs = pairs + 1u;
        // The coverage gate (#944): a light whose WHOLE range projects under the threshold casts no
        // pages — it still shades, and the reader finds nothing and returns lit.
        if light_gated(light) {
            culled = culled + 1u;
            continue;
        }
        if light_distant(light) {
            distant = distant + 1u;
        }
        _ = mark_local(light, world, local_wanted);
    }
    if pairs != 0u {
        atomicAdd(&tally[TALLY_PAIRS], pairs);
    }
    if culled != 0u {
        atomicAdd(&tally[TALLY_CULLED], culled);
    }
    if distant != 0u {
        atomicAdd(&tally[TALLY_DISTANT], distant);
    }
}

// The projected radius of a light's range sphere, in screen pixels — the whole reach of the light,
// not the lit part of it, so the gate errs toward casting. Camera-dependent and pixel-independent:
// the same number every sample computes.
fn coverage_pixels(light: u32) -> f32 {
    let record = lights[light];
    let distance = max(length(record.position - pages.eye_and_base.xyz), 0.05);
    let focal = view.clip_from_view[1][1];
    return record.range * abs(focal) * view.viewport.y / (2.0 * distance);
}

// Whether the light is past the distance the settings let it cast from.
fn light_out_of_reach(light: u32) -> bool {
    if pages.density.w <= 0.0 {
        return false;
    }
    let record = lights[light];
    let distance = length(record.position - pages.eye_and_base.xyz);
    return distance > record.range * pages.density.w;
}

// Whether this light is DISTANT: its whole range projects under `page_min_pixels`, so it gets ONE
// page per cube face rather than a chain of them.
fn light_distant(light: u32) -> bool {
    return light_single_level(light)
        || (pages.density.y > 0.0 && coverage_pixels(light) < pages.density.y);
}

// The world size of one screen pixel at `depth` along the view axis. Mirrors what `mark_pixel`
// computes for its own sample, so the two cannot answer differently about the same distance.
fn pixel_world(depth: f32) -> f32 {
    let focal = view.clip_from_view[1][1];
    if abs(focal) < 1e-9 {
        return 0.0;
    }
    return 2.0 * abs(depth) / (focal * max(view.viewport.y, 1.0));
}

// Whether the FINEST level any pixel could ask this light for is already the coarsest one it has.
fn light_single_level(light: u32) -> bool {
    let record = lights[light];
    if record.range <= 0.0 {
        return false;
    }
    // The nearest the light's sphere can come to the camera, and so the
    // smallest a screen pixel covering it can be.
    let near = max(length(record.position - pages.eye_and_base.xyz) - record.range, 0.0);
    let wanted = pixel_world(near) * pages.density.x;
    if wanted <= 0.0 {
        return false;
    }
    // Measured out at the range, which is the furthest a receiver of
    // this light can sit and so the finest level it can ask for.
    return page_level(record.range, wanted) >= pages.chain.z - 1u;
}

// Whether this light casts nothing at all.
fn light_gated(light: u32) -> bool {
    return light_out_of_reach(light);
}

// Paints the page each pixel chose, over the frame's final colour.
@compute @workgroup_size(MARK_GROUP, MARK_GROUP, 1)
fn paint_view(@builtin(global_invocation_id) id: vec3<u32>) {
    if pages.sampling.z == 0u {
        return;
    }
    let pixel = id.xy;
    let size = vec2<u32>(view.viewport.xy);
    if pixel.x >= size.x || pixel.y >= size.y {
        return;
    }
    let depth = textureLoad(depth_tex, vec2<i32>(pixel), 0);
    if depth <= 0.0 {
        return;
    }
    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / view.viewport.xy;
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let hom = pages.world_from_clip * vec4<f32>(ndc, 1.0);
    if abs(hom.w) < 1e-9 {
        return;
    }
    let world = hom.xyz / hom.w;
    let view_pos = view.view_from_world * vec4<f32>(world, 1.0);
    let focal = view.clip_from_view[1][1];
    var wanted = 0.0;
    if abs(focal) > 1e-9 {
        wanted = 2.0 * abs(view_pos.z) / (focal * max(view.viewport.y, 1.0));
    }
    wanted = wanted * pages.density.x;
    // The same bias the marking applied, or the view paints pages the
    // marking never chose — the failure `sun_page_for` was split to end.
    let bias = atomicLoad(&rank_state[rank_base() + RANK_BIAS]);

    // 🔴 One page per pixel, and the sun wins when there is one: a pixel is lit by many lights, and
    // painting the last one walked would make the view depend on the light list's order.
    if pages.sun.w > 0.5 {
        paint_page(
            pixel,
            sun_page_for(pages.sampling.y, world, wanted * exp2(f32(bias >> 8u))),
        );
        return;
    }
    if view.dimensions.w == 0u {
        return;
    }
    let cell = cluster_of_ndc(view, ndc, view_pos.z);
    let record = cells[cluster_index(cell, view.dimensions)];
    let count = record.point_count + record.spot_count;
    for (var i = 0u; i < count; i = i + 1u) {
        let slot = record.offset + i;
        if slot >= arrayLength(&indices) {
            break;
        }
        let light = indices[slot];
        if light >= pages.strides.w {
            continue;
        }
        // The same gate the marking applied (#944).
        if light_gated(light) {
            continue;
        }
        paint_page(
            pixel,
            local_page_for(light, world, wanted * exp2(f32(bias & 0xffu))),
        );
        return;
    }
}

// Writes the debug colour, when the view is on and a page was chosen.
fn paint_page(pixel: vec2<u32>, painted: vec2<u32>) {
    if pages.sampling.z == 0u || painted.x == NO_PAGE {
        return;
    }
    let color = vec4<f32>(page_color(painted.x, painted.y), 1.0);
    // The block of output pixels this depth pixel covers, filled whole.
    let scale = pages.paint.xy;
    let size = vec2<u32>(pages.paint.zw);
    let lo = vec2<u32>(floor(vec2<f32>(pixel) * scale));
    let hi = min(vec2<u32>(ceil(vec2<f32>(pixel + vec2<u32>(1u)) * scale)), size);
    for (var y = lo.y; y < hi.y; y = y + 1u) {
        for (var x = lo.x; x < hi.x; x = x + 1u) {
            textureStore(color_out, vec2<i32>(vec2<u32>(x, y)), color);
        }
    }
}

// Ages this view's table entries, and evicts the ones nothing has asked for in `max_age` frames.
@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn age_view(@builtin(global_invocation_id) id: vec3<u32>) {
    let within = id.x;
    if within >= view_span() { return; }
    let entry = view_base() + within;
    // The receiver bound (#940) is a per-FRAME quantity: this pass is the one thread per entry that
    // already runs first, so the reset rides it. Zero means "no receiver recorded", which every
    // reader treats as "never reject".
    let stored = atomicLoad(&table_cells[entry * PAGE_CELL]);
    if stored == PAGE_ABSENT { return; }

    // A rebuild empties the view outright — the pool's shape changed
    // under it, so a slot in an old entry names a page in a new atlas.
    if pages.life.z == 0u {
        let age = page_age(entry);
        // Unsigned, so a frame index that ran backwards (a rebuild, a wrap) reads as enormous and
        // evicts. That is the safe way for this comparison to be wrong.
        if pages.life.x - age <= pages.life.y {
            atomicAdd(&counters[11], 1u);
            return;
        }
    }
    atomicStore(&table_cells[entry * PAGE_CELL], PAGE_ABSENT);
    page_release(stored - 1u);
    atomicAdd(&counters[12], 1u);
}

// Whether the frame marked this page, without claiming the bit.
fn mark_test(index: u32) -> bool {
    let word = index / 32u;
    if word >= arrayLength(&marks) {
        return false;
    }
    return (atomicLoad(&marks[word]) & (1u << (index % 32u))) != 0u;
}

// The rank of a table entry, decoded from its index within the view —
// the inverse of `local_page_for`/`sun_page_for`'s address arithmetic.
// The seat passes walk entries, and an entry does not carry its level.
fn entry_rank(within: u32) -> u32 {
    let sun_base = pages.sampling.y * pages.strides.z;
    if within >= sun_base {
        let cell = pages.strides.x * pages.strides.x;
        let level = min((within - sun_base) / cell, pages.chain.w - 1u);
        return rank_sun(level);
    }
    // Which level of a face's chain the offset falls in: the running
    // sum `level_base` climbs from the floor, at most `chain.z` steps.
    let face = (within % pages.strides.z) % pages.strides.y;
    var level = local_level_floor(pages.chain.y);
    var next = level_base(level) + level_side(level) * level_side(level);
    while level + 1u < pages.chain.z && face >= next {
        level = level + 1u;
        next = level_base(level) + level_side(level) * level_side(level);
    }
    return rank_local(level);
}

// Turns the demand histogram into a seating plan: how deep down the ranks this view's slice
// reaches. One thread — RANKS is 32 and the loop IS the prefix sum; a parallel scan here would cost
// more to coordinate than it computes.
@compute @workgroup_size(1, 1, 1)
fn plan_view() {
    let base = rank_base();
    let budget = pages.pool.w;
    var used = 0u;
    var cutoff = RANKS;
    var quota = 0u;
    for (var r = 0u; r < RANKS; r = r + 1u) {
        let d = atomicLoad(&rank_state[base + r]);
        if used + d > budget {
            cutoff = r;
            quota = budget - used;
            used = budget;
            break;
        }
        used = used + d;
    }
    atomicStore(&rank_state[base + RANK_CUTOFF], cutoff);
    atomicStore(&rank_state[base + RANK_QUOTA], quota);
    atomicStore(&rank_state[base + RANK_SPARE], budget - used);
    atomicStore(&counters[15], cutoff);
}

// Clears the seats the plan did not fund.
@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn preempt_view(@builtin(global_invocation_id) id: vec3<u32>) {
    let within = id.x;
    if within >= view_span() { return; }
    let entry = view_base() + within;
    let stored = atomicLoad(&table_cells[entry * PAGE_CELL]);
    if stored == PAGE_ABSENT { return; }
    let base = rank_base();
    if mark_test(entry) {
        let cutoff = atomicLoad(&rank_state[base + RANK_CUTOFF]);
        let rank = entry_rank(within);
        if rank < cutoff { return; }
        if rank == cutoff {
            // Same take-then-test as `page_alloc`: the old value says whether the take was funded.
            let quota = atomicSub(&rank_state[base + RANK_QUOTA], 1u);
            if quota != 0u && quota <= pages.pool.w { return; }
            atomicAdd(&rank_state[base + RANK_QUOTA], 1u);
        }
    } else {
        // Unrequested this frame. It stays as CACHE only while the slice has room after the frame's
        // own demand is seated — `age_view` already dropped what went stale, this is the pressure
        // valve on the rest.
        let spare = atomicSub(&rank_state[base + RANK_SPARE], 1u);
        if spare != 0u && spare <= pages.pool.w { return; }
        atomicAdd(&rank_state[base + RANK_SPARE], 1u);
    }
    atomicStore(&table_cells[entry * PAGE_CELL], PAGE_ABSENT);
    page_release(stored - 1u);
    atomicAdd(&counters[14], 1u);
}

// Seats this frame's marked pages: everything under the cutoff gets a slot, the cutoff rank
// competes for what the residents left of the quota, and everything past it is DENIED and counted —
// the panel's answer to who the slice turned away.
@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn adopt_view(@builtin(global_invocation_id) id: vec3<u32>) {
    let within = id.x;
    if within >= view_span() { return; }
    let entry = view_base() + within;
    if !mark_test(entry) { return; }
    if atomicLoad(&table_cells[entry * PAGE_CELL]) != PAGE_ABSENT { return; }
    let base = rank_base();
    let cutoff = atomicLoad(&rank_state[base + RANK_CUTOFF]);
    let rank = entry_rank(within);
    if rank > cutoff {
        atomicAdd(&counters[13], 1u);
        return;
    }
    if rank == cutoff {
        let quota = atomicSub(&rank_state[base + RANK_QUOTA], 1u);
        if quota == 0u || quota > pages.pool.w {
            atomicAdd(&rank_state[base + RANK_QUOTA], 1u);
            atomicAdd(&counters[13], 1u);
            return;
        }
    }
    let slot = page_alloc();
    if slot == PAGE_MISS {
        atomicAdd(&counters[13], 1u);
        return;
    }
    page_stamp(entry, slot, pages.life.x);
    atomicAdd(&counters[8], 1u);
}

// Moves the resolution bias one step per frame toward the coarsest marking that fits the slice
// (#943).
@compute @workgroup_size(1, 1, 1)
fn bias_view() {
    let base = rank_base();
    let word = atomicLoad(&rank_state[base + RANK_BIAS]);
    var local_bias = word & 0xffu;
    var sun_bias = word >> 8u;
    let cutoff = atomicLoad(&rank_state[base + RANK_CUTOFF]);
    var patience = atomicLoad(&rank_state[base + RANK_PATIENCE]);
    let budget = pages.pool.w;

    // 🔴 Read on BOTH branches now. The raise used to step by one and wait for the next frame to see
    // whether that was enough, so a scene needing four steps took four frames to stop denying and
    // up to ninety-six to give them back.
    var sun_demand = 0u;
    var local_demand = 0u;
    var total = 0u;
    for (var r = 0u; r < RANKS; r = r + 1u) {
        let d = atomicLoad(&rank_state[base + r]);
        total = total + d;
        if r < pages.chain.w {
            sun_demand = sun_demand + d;
        } else {
            local_demand = local_demand + d;
        }
    }

    // 🔴 The allocator's own ledger, taken here because this is the one single-threaded pass that
    // runs AFTER `adopt_view` — so it reports what the seating actually left, not what the plan
    // intended.
    atomicStore(&counters[17], atomicLoad(&alloc[alloc_base()]));
    atomicStore(&counters[18], atomicLoad(&alloc[alloc_base() + 1u]));
    atomicStore(&counters[19], total);

    if cutoff < RANKS {
        // ⚠️ Four pages become one per step is the OPTIMISTIC estimate, and it is chosen on
        // purpose. Raising too little costs one more frame of denials; raising too much costs blur
        // the player sees. The error belongs on the low side.
        var local_room = LOCAL_BIAS_MAX - min(local_bias, LOCAL_BIAS_MAX);
        var extra = local_room;
        for (var k = 1u; k <= local_room; k = k + 1u) {
            if sun_demand + (local_demand >> (2u * k)) <= budget {
                extra = k;
                break;
            }
        }
        local_bias = local_bias + extra;
        let local_left = local_demand >> (2u * extra);
        // The sun pays only when the cut landed among ITS ranks, and
        // only once the lamps have given everything they have.
        if local_bias >= LOCAL_BIAS_MAX && cutoff < pages.chain.w {
            let sun_room = SUN_BIAS_MAX - min(sun_bias, SUN_BIAS_MAX);
            var sun_extra = sun_room;
            for (var k = 1u; k <= sun_room; k = k + 1u) {
                if (sun_demand >> (2u * k)) + local_left <= budget {
                    sun_extra = k;
                    break;
                }
            }
            sun_bias = sun_bias + sun_extra;
        }
        patience = 0u;
    } else {
        patience = patience + 1u;
        // ⚠️ Growth is FOUR times a level here, the pessimistic estimate, for the same reason
        // inverted: an unwind that over-reaches is the blur coming straight back next frame.
        let others = total - sun_demand;
        var back = 0u;
        for (var k = 1u; k <= sun_bias; k = k + 1u) {
            if others + (sun_demand << (2u * k)) <= budget {
                back = k;
            } else {
                break;
            }
        }
        if back > 0u {
            sun_bias = sun_bias - back;
            patience = 0u;
        } else {
            let rest = total - local_demand;
            var local_back = 0u;
            for (var k = 1u; k <= local_bias; k = k + 1u) {
                if rest + (local_demand << (2u * k)) <= budget {
                    local_back = k;
                } else {
                    break;
                }
            }
            if local_back > 0u {
                local_bias = local_bias - local_back;
                patience = 0u;
            } else if patience >= PATIENCE_FRAMES && (sun_bias > 0u || local_bias > 0u) {
                // Trial: the proof is unavailable — coarse levels do not
                // quadruple — so probe, and let the raise arbitrate.
                if sun_bias > 0u {
                    sun_bias = sun_bias - 1u;
                } else {
                    local_bias = local_bias - 1u;
                }
                patience = 0u;
            }
        }
    }
    atomicStore(&rank_state[base + RANK_PATIENCE], patience);
    let packed = local_bias | (sun_bias << 8u);
    atomicStore(&rank_state[base + RANK_BIAS], packed);
    // What the pool is converging to, for the panel.
    atomicStore(&counters[4], packed);
}

