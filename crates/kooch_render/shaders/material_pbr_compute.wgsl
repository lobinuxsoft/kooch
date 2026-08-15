// material_pbr_compute.wgsl — the compute shading body (#824).
//
// The same shading `material_pbr_default.wgsl` does, moved out of the
// fragment stage so a workgroup can own a screen tile and read that
// tile's light list once instead of once per pixel.
//
// CONCATENATED by the same `compose_material_shader` the fragment body
// uses, so everything below the entry point is shared, line for line:
// `visibility_buffer_resolve.wgsl` (the R64 read, groups 0), the
// barycentric reconstruction, the contact-shadow march, and Inti.
//
// # Why this is legal in a compute stage
//
// The fragment path samples with `textureSampleGrad` and feeds it the
// derivatives the surface reconstruction computes analytically —
// automatic quad derivatives are wrong here anyway, because two
// fragments in the same 2x2 quad can reconstruct from different
// triangles. A call that takes its gradients explicitly needs no
// fragment stage, so this path samples exactly the same three maps.
//
// (The R32 compute path in `meshlet_deferred.wgsl` says the opposite in
// a comment and shades from the material scalars alone. That comment is
// wrong, and #824 is the issue that found it out.)
//
// # What the measurement asked for
//
// `raster + shade` is 31.41 ms on the OneXFly and is **not ALU-bound**:
// deleting the whole specular model bought 10 % (#821). The grid is not
// over-listing either — 14.9 lights mean against 12 that reach a point
// (#820). What was left is that every pixel fetches its froxel's ~15
// `IntiLight` records out of a storage buffer, and every pixel of a tile
// fetches the same ones.

struct MaterialParams {
    base_color: vec4<f32>,
    // x metallic, y roughness, z emissive, w pad.
    metallic_roughness_emissive_pad: vec4<f32>,
    texture_indices: vec4<u32>,
}

// Group 0 holds the vbuf (0), camera (1), screen (2) and the contact
// shadow's UBO (3) + depth (4) — all declared by the concatenated
// prefix. The shading targets are this path's own, and the first free
// indices.
//
// At `screen.shading_rate == 1` `color_out` is the screen; at 2 it is a
// half-resolution texture the upsample pass reads back (#825).
@group(0) @binding(5) var color_out: texture_storage_2d<rgba8unorm, write>;

// #825 — which surface each shaded sample came from, as
// `visible_slot + 1` (0 means the sample shaded nothing). The upsample
// pass compares it against the full-resolution vbuf to decide which
// samples a pixel is allowed to blend, which is what keeps silhouettes
// sharp when the lighting is not. Written only at half rate.
@group(0) @binding(6) var shaded_ids: texture_storage_2d<r32uint, write>;

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

@group(4) @binding(0) var albedo_tex: texture_2d<f32>;
@group(4) @binding(1) var normal_tex: texture_2d<f32>;
@group(4) @binding(2) var metal_rough_tex: texture_2d<f32>;
@group(4) @binding(3) var material_sampler: sampler;

// Tile edge in pixels. 16x16 = 256 threads, one wavefront's worth of
// work per lane on AMD at wave32 and the size every tiled-deferred
// reference lands on. It is also small enough that a tile usually sits
// inside one froxel column: at 1280x720 the grid's cells are ~75x80 px.
const TILE_SIZE: u32 = 16u;
const TILE_THREADS: u32 = TILE_SIZE * TILE_SIZE;

// 🔴 A CAP, and what happens when it is hit.
//
// A tile's pixels do not all sit in one froxel. They share a column of
// the grid almost always (the cells are wider than the tile), but depth
// varies across a tile — and at a silhouette it varies a lot, because
// the near surface and whatever is behind it land many z-slices apart.
// So the tile draws its lights from a BLOCK of cells, `[min..max]` per
// axis, and the block is only small because real tiles are mostly one
// continuous surface.
//
// When a tile's block exceeds either cap, the tile shades from the
// storage buffer exactly as the fragment path does. That is slower and
// it is correct — which is the right way around, because a tile that
// straddles a silhouette is rare and a tile that renders wrong is not
// something a capture would ever show us.
const MAX_TILE_CELLS: u32 = 16u;
const MAX_TILE_LIGHTS: u32 = 384u;

// The block of froxels this tile covers, reduced from its own threads.
// Held per axis: a linear cell index cannot be reduced this way, because
// two pixels one slice apart differ by 1 in z and by an arbitrary amount
// in the linear index.
var<workgroup> tile_cell_min: array<atomic<u32>, 3>;
var<workgroup> tile_cell_max: array<atomic<u32>, 3>;

// The tile's light indices, one contiguous run per cell of the block.
var<workgroup> tile_lights: array<u32, MAX_TILE_LIGHTS>;
var<workgroup> tile_cell_start: array<u32, MAX_TILE_CELLS>;
var<workgroup> tile_cell_len: array<u32, MAX_TILE_CELLS>;

// 🔴 #826 — the part of each light needed to WEIGH it, in workgroup
// memory. `xyz` is the world position and `w` the luminous intensity
// already divided by 4π; the range rides alongside.
//
// This is what #824 stopped one step short of. It cached the tile's
// light *indices* — four bytes each — and every pixel still fetched the
// whole 80-byte `IntiLight` out of the storage buffer for every light
// in its froxel. Fifteen of those is 1.2 KB per pixel, and #824 bought
// 6.6 % because it removed the four bytes and left the 1200.
//
// Sampling needs a weight per light per pixel, so doing it off the
// storage records would have kept exactly that traffic and paid for the
// weights on top. Twenty bytes per light, read once per tile, is what
// makes the weighting cheaper than the evaluation it replaces — which
// is the only reason any of this is faster.
var<workgroup> tile_light_pos: array<vec4<f32>, MAX_TILE_LIGHTS>;
var<workgroup> tile_light_range: array<f32, MAX_TILE_LIGHTS>;
// Set for a light that owns a shadow map. Those are never sampled — see
// `tile_choose`.
var<workgroup> tile_light_caster: array<u32, MAX_TILE_LIGHTS>;

// 🔴 #826 — what the tile chose, per cell of its block.
//
// `tile_pick` holds slots into `tile_lights`; `tile_pick_scale` the
// reciprocal of the probability each was chosen with, which is 1 for a
// shadow caster because a caster is not chosen at all. Every pixel of a
// cell evaluates this list and nothing else, so the per-pixel cost stops
// depending on how many lights reach the cell.
const MAX_TILE_PICKS: u32 = 16u;
// Half the list, so a cell whose casters fill their half still samples.
const MAX_TILE_STRATA: u32 = 8u;
var<workgroup> tile_pick: array<u32, MAX_TILE_CELLS * MAX_TILE_PICKS>;
var<workgroup> tile_pick_scale: array<f32, MAX_TILE_CELLS * MAX_TILE_PICKS>;
var<workgroup> tile_pick_count: array<atomic<u32>, MAX_TILE_CELLS>;

// One real surface point inside each cell, and which thread published
// it. The weights are computed against this rather than against the
// pixel being shaded — that is the whole saving, and `tile_light_reach`
// documents why it costs no correctness.
//
// Elected by `atomicMin` over the thread id rather than taken from
// whichever thread arrives last. A race here would pick a different
// point each frame with the camera still, and #826 already learnt what
// a per-frame choice of light looks like on a screen.
var<workgroup> tile_rep_owner: array<atomic<u32>, MAX_TILE_CELLS>;
var<workgroup> tile_rep_pos: array<vec3<f32>, MAX_TILE_CELLS>;

// Set when the block did not fit. Read by every thread after the load
// barrier; written only by lane 0, before it.
var<workgroup> tile_overflow: u32;

/// Inti's shading, with the punctual lights taken from workgroup memory
/// instead of the cluster index buffer.
///
/// 🔴 Same lights, same order, same arithmetic as `inti_shade`. The only
/// difference is where `inti_cluster_indices` was read from — which is
/// the entire point of the issue, and the reason the image must not
/// move by one bit.
fn shade_from_tile(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    frag_coord: vec2<f32>,
    flags: u32,
    start: u32,
    len: u32,
) -> vec3<f32> {
    let surf = inti_surface(world_position, n, base_color, metallic, roughness, flags);

    var radiance = vec3<f32>(0.0);
    // Directional lights are not in the grid — they reach every cell, so
    // a cell listing them would say nothing. They are the light buffer's
    // leading entries and this walk is unchanged.
    for (var i = 0u; i < inti.directional_count; i = i + 1u) {
        radiance += inti_light_contribution(surf, inti_lights[i], frag_coord);
    }
    // `KOOCH_LIGHT_LIMIT`, the same cap `inti_clustered_lights` applies
    // to the storage walk. Both paths have to honour it or an A/B
    // between them stops being one.
    var walk = len;
    if (inti.light_limit != 0u) {
        walk = min(walk, inti.light_limit);
    }
    for (var i = 0u; i < walk; i = i + 1u) {
        radiance += inti_light_contribution(
            surf, inti_lights[tile_lights[start + i]], frag_coord);
    }
    radiance += inti_ambient(n, surf.diffuse_color, surf.f0, surf.f_ab);
    return radiance;
}

/// What this light is worth **anywhere near** `p`, before any BRDF
/// (#826).
///
/// Luminance times distance attenuation, off the workgroup copy instead
/// of the 80-byte record. Roughly six arithmetic operations against a
/// full `inti_light_contribution`, which is the ratio the whole issue
/// trades on.
///
/// # 🔴 Why the cosine is gone
///
/// The first draft multiplied by `n·L` and was evaluated per pixel, with
/// that pixel's own normal. Moving the choice to the tile means one
/// point stands in for a whole froxel — and a froxel spans surfaces
/// facing different ways, so there is no normal that is honest for all
/// of them.
///
/// Keeping a representative normal would have been worse than dropping
/// it. The estimator stays unbiased no matter how crude the weight is,
/// **as long as no light that contributes is given a weight of zero** —
/// each pick is divided by the probability it was made with, so a bad
/// ranking costs variance and nothing else. A cosine against the wrong
/// normal returns exactly that forbidden zero, for every light on the
/// far side of a plane the pixel does not share.
///
/// So the weight ranks by "how much light arrives in this neighbourhood"
/// and lets the BRDF decide what the surface does with it. Cruder, and
/// the crudeness is the safe direction.
fn tile_light_reach(slot: u32, p: vec3<f32>) -> f32 {
    let rec = tile_light_pos[slot];
    let offset = rec.xyz - p;
    return rec.w * inti_distance_attenuation(dot(offset, offset), tile_light_range[slot]);
}

/// A uniform number in [0,1) for one (light, pixel, stratum) triple.
///
/// 🔴 Keyed on the light's GLOBAL index, never on its position in the
/// froxel's run — and that is the fix for the flicker, not a detail of
/// the hash.
///
/// The grid fills a cell's run with `atomicAdd` (see
/// `cluster_raster.wgsl::write_index`), so the slot a light lands in is
/// the order its thread happened to arrive. That order changes **every
/// frame**, with nothing in the scene moving. Measured on the parity
/// scene, two consecutive renders of an identical view:
///
/// | | pixels that changed | worst channel |
/// |---|---|---|
/// | walking every light | 1 | 1 |
/// | `KOOCH_LIGHT_LIMIT=2` | 10 098 | 164 |
///
/// So the froxel shimmer the limit produced on the device was never
/// about crossing cell boundaries — a still camera shimmers on its own,
/// because "the first two of the list" is a different pair of lights
/// each frame. Anything that reads the run's *order* inherits that, and
/// the first draft of this function did: it stratified the cumulative
/// weight, which is an order.
///
/// Keying on the global index makes the choice a property of the light
/// and the pixel. The run can be permuted at will and the same lights
/// come out.
fn tile_light_random(light_index: u32, pixel: vec2<u32>, stratum: u32) -> f32 {
    var h = light_index * 747796405u + 2891336453u;
    h = h ^ ((pixel.x * 2654435761u) + (pixel.y * 40503u));
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = h ^ (stratum * 1013904223u);
    h = (h >> 22u) ^ h;
    // 24 bits is every value an f32 can hold exactly below 1.0.
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

/// The cell's lights, `inti.light_samples` of them, chosen in proportion
/// to what they contribute — **once for the whole tile** (#826).
///
/// # Why this is not per pixel, in the numbers that decided it
///
/// The first version ran this race in every pixel, and the device
/// measured what that costs. Solving the three runs for the cost of one
/// weight against the cost of one full light evaluation:
///
/// | samples | `shade: compute` | weights per pixel |
/// |---|---|---|
/// | 0 (walk all 15) | 12.624 ms | 0 |
/// | 2 | 10.482 ms | 45 |
/// | 4 | 16.837 ms | 75 |
///
/// gives **a weight at 0.196 of an evaluation** — a fifth, not the
/// fifteenth the design assumed. At that ratio `(K+1) x 15` weights
/// costs more than the twelve evaluations it removes as soon as K
/// reaches 4, which is exactly the non-monotonic curve above. The
/// technique was not wrong; the estimator was in the wrong loop.
///
/// Here the `len` weights are paid once per cell by one thread, in
/// parallel with the other cells and strata, instead of once per stratum
/// per pixel. What survives into the pixel is a list of picks and their
/// scales, so shading costs `picks` evaluations and **no weights at
/// all**.
///
/// # What the tile shares, and the seam that comes with it
///
/// Every pixel of a cell now evaluates the same lights. That is the
/// trade, and it is the same one HypeHype's Stratified Tile-Based
/// Lighting makes (SIGGRAPH 2025) — their small tile is 16 px, which is
/// this workgroup exactly. The error stops being per-pixel noise and
/// becomes a discontinuity at cell boundaries.
///
/// ⚠️ For an engine with no temporal pass that is the **better** artefact
/// and the choice is deliberate: a block that is slightly wrong is
/// spatially coherent and reads as shading, where per-pixel noise reads
/// as dirt. The contact shadows already demonstrate the other side of
/// this on the device.
///
/// # The exponential race, and why not the cheaper thing
///
/// Each light draws its own number and races on `-log(u) / w`; the
/// smallest wins the stratum. That picks light *i* with probability
/// `w_i / w_sum` — weighted reservoir sampling — and it is an
/// **argmin over independent per-light keys**, so permuting the run
/// cannot change the winner.
///
/// The first draft did the obvious cheaper thing: walk the cumulative
/// weight once and take the sample of every stratum on the way past. One
/// pass, one random number, no per-stratum loop. It was also wrong, and
/// not subtly — a cumulative walk *is* an order, and this run's order
/// is whatever the grid's atomics produced that frame. See
/// `tile_light_random` for the measurement that caught it.
///
/// One thread runs one stratum, so the `strata` races happen at once
/// across the workgroup rather than one after another. A cell with four
/// strata occupies four threads for the length of one walk.
///
/// ⚠️ Two strata can pick the same light and it is then evaluated twice.
/// Correct, because each carries its own `1/K` share, and rare unless
/// one light dominates the froxel.
///
/// # 🔴 The floor under every weight, and the bias it buys off
///
/// A froxel is a volume. A light whose range cuts through it reaches
/// some of the cell and not the representative point, so its weight
/// comes out zero while its contribution to a real pixel does not — the
/// one failure the estimator cannot absorb, because a light that is
/// never picked is never divided back up.
///
/// So every light in the run is given a share of the probability
/// regardless of what it scored: a thirty-second of the budget, spread
/// evenly. That is defensive sampling, it costs a little variance, and
/// it turns "may silently lose a light" into "may occasionally spend a
/// sample on a dim one". When nothing scores at all the mixture is all
/// there is and the choice becomes uniform, which is correct rather than
/// a fallback: if the representative sees no light and a pixel does,
/// uniform is the only honest prior.
///
/// # Why the result is not just darker
///
/// Each evaluated light is scaled by `w_sum / (K * w)` — the reciprocal
/// of the probability it was picked with. A light twice as important is
/// picked twice as often and counted half as much, so the average over
/// the strata estimates the sum over all the lights. That is the whole
/// difference from `KOOCH_LIGHT_LIMIT`, which keeps a prefix and scales
/// it by nothing: on the parity scene, two sampled lights land 3.6 %
/// from the full walk\'s brightness where two truncated ones land 83 %
/// away.
/// `cell` indexes this tile's block; `seed` is the same froxel's index in
/// the grid.
///
/// 🔴 The two are not interchangeable and the seam depends on which one
/// the race is drawn from. A froxel is about 75 x 80 px and a tile is 16,
/// so several tiles cover one froxel and each numbers it differently
/// inside its own block. Seeded on the local number, neighbouring tiles
/// draw different lights from the same list and the discontinuity lands
/// every 16 px. Seeded on the grid's index they draw the same ones, and
/// what is left shows only where the light list genuinely changes.
fn tile_choose(cell: u32, seed: u32, stratum: u32, strata: u32) {
    let start = tile_cell_start[cell];
    let len = tile_cell_len[cell];
    if (len == 0u) {
        return;
    }
    let rep = tile_rep_pos[cell];

    // Pass 1 — the total, and the mixture that keeps every light
    // reachable. Casters are excluded from both: they are already in the
    // list and sampling them again would double them.
    var raw_sum = 0.0;
    var sampled = 0u;
    for (var i = 0u; i < len; i = i + 1u) {
        if (tile_light_caster[start + i] != 0u) {
            continue;
        }
        raw_sum += tile_light_reach(start + i, rep);
        sampled = sampled + 1u;
    }
    if (sampled == 0u || stratum >= min(strata, sampled)) {
        return;
    }
    // A thirty-second of the budget, spread evenly — or the whole of it
    // when nothing scored, which makes the choice uniform.
    let share = select(1.0, raw_sum / (f32(sampled) * 32.0), raw_sum > 0.0);
    let w_sum = raw_sum + share * f32(sampled);

    let used = min(strata, sampled);
    let inv_used = 1.0 / f32(used);
    var best_key = 3.402823e38;
    var best_slot = 0xffffffffu;
    var best_w = 0.0;
    for (var i = 0u; i < len; i = i + 1u) {
        let slot = start + i;
        if (tile_light_caster[slot] != 0u) {
            continue;
        }
        let w = tile_light_reach(slot, rep) + share;
        // 🔴 One draw per light, ROTATED by the stratum, not a fresh
        // draw per stratum. `fract(u + k/K)` is still uniform, so each
        // race on its own is still proportional to `w` — but a light
        // that won with `u` near 1 gets a low one next time, so the
        // strata stop landing on the same light. That is stratification's
        // variance reduction without stratification's dependence on the
        // run's order.
        //
        // Seeded on the froxel, not on a pixel: this runs once for all of
        // them. `tile_lights[slot]` is the light's global index, which is
        // what makes the winner independent of where the grid's atomics
        // happened to put it this frame.
        let u = fract(tile_light_random(tile_lights[slot], vec2<u32>(seed, 0u), 0u)
            + f32(stratum) * inv_used);
        // `-log(u) / w` is an exponential of rate `w`; the smallest of a
        // set of them belongs to index `i` with probability `w_i /
        // w_sum`, which is the distribution this needs.
        let key = -log(max(u, 1e-7)) / w;
        if (key < best_key) {
            best_key = key;
            best_slot = slot;
            best_w = w;
        }
    }
    if (best_slot == 0xffffffffu) {
        return;
    }
    let at = atomicAdd(&tile_pick_count[cell], 1u);
    if (at < MAX_TILE_PICKS) {
        tile_pick[cell * MAX_TILE_PICKS + at] = best_slot;
        tile_pick_scale[cell * MAX_TILE_PICKS + at] = w_sum * inv_used / best_w;
    }
}

/// Shading from what the tile chose: `picks` evaluations, no weights.
///
/// The scale on each pick is the reciprocal of the probability it was
/// chosen with, so a light twice as important is chosen twice as often
/// and counted half as much and the average estimates the sum over all
/// the lights. That is the whole difference from `KOOCH_LIGHT_LIMIT`,
/// which keeps a prefix and scales it by nothing: on the parity scene,
/// two sampled lights land 3.6 % from the full walk's brightness where
/// two truncated ones land 83 % away.
fn shade_picked_from_tile(
    world_position: vec3<f32>,
    n: vec3<f32>,
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    frag_coord: vec2<f32>,
    flags: u32,
    cell: u32,
) -> vec3<f32> {
    let surf = inti_surface(world_position, n, base_color, metallic, roughness, flags);

    var radiance = vec3<f32>(0.0);
    for (var i = 0u; i < inti.directional_count; i = i + 1u) {
        radiance += inti_light_contribution(surf, inti_lights[i], frag_coord);
    }

    let picks = min(atomicLoad(&tile_pick_count[cell]), MAX_TILE_PICKS);
    for (var m = 0u; m < picks; m = m + 1u) {
        let at = cell * MAX_TILE_PICKS + m;
        radiance += inti_light_contribution(
            surf, inti_lights[tile_lights[tile_pick[at]]], frag_coord)
            * tile_pick_scale[at];
    }

    radiance += inti_ambient(n, surf.diffuse_color, surf.f0, surf.f_ab);
    return radiance;
}

@compute @workgroup_size(16, 16, 1)
fn cs_shade_tile(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    // 🔴 NO EARLY RETURN ANYWHERE ABOVE THE LAST BARRIER. A thread that
    // leaves the function skips the `workgroupBarrier`s its neighbours
    // are still waiting on, which is undefined behaviour and in practice
    // a hang. Pixels outside the screen, pixels of another material and
    // background pixels all walk the whole function and simply write
    // nothing.
    if (lid < 3u) {
        atomicStore(&tile_cell_min[lid], 0xffffffffu);
        atomicStore(&tile_cell_max[lid], 0u);
    }
    if (lid == 0u) {
        tile_overflow = 0u;
    }
    workgroupBarrier();

    // The sample this thread owns, and the quad of pixels it stands for.
    // At full rate the quad is one pixel and `pixel == sample`.
    let sample = gid.xy;
    let rate = screen.shading_rate;
    let origin = sample * rate;

    // 🔴 The representative is chosen from the VISIBILITY BUFFER ALONE —
    // the first covered pixel of the quad, in a fixed order — and never
    // from this dispatch's material.
    //
    // Every material dispatch runs over every sample, so if each picked
    // the pixel of its own material the two would write the same texel
    // and the last one to run would win. Choosing from the vbuf makes
    // the representative a property of the frame: exactly one dispatch
    // finds `material_id` equal, exactly one writes.
    //
    // It also removes a hole the upsample would otherwise have. A
    // covered pixel's own quad always contains a covered pixel — itself
    // — so its own quad's sample was shaded, and that sample is always
    // one of the four the upsample considers. There is no covered pixel
    // anywhere on screen with nothing to read from.
    var pixel = origin;
    var visibility = 0lu;
    var covered = false;
    let quad = rate * rate;
    for (var q = 0u; q < quad; q = q + 1u) {
        let cand = origin + vec2<u32>(q % rate, q / rate);
        if (cand.x < screen.size.x && cand.y < screen.size.y) {
            let packed = textureLoad(vbuf64, cand).x;
            // `packed >> 32 == 0` is the background sentinel under
            // reversed-Z, the same test `resolve_material_depth.wgsl`
            // makes before it discards.
            if ((packed >> 32u) != 0lu) {
                pixel = cand;
                visibility = packed;
                covered = true;
                break;
            }
        }
    }
    // The pixel centre — the same coordinate the fragment path's
    // `@builtin(position)` carries, so the froxel lookup and the
    // contact-shadow dither agree between the two paths.
    let frag_coord = vec2<f32>(pixel) + vec2<f32>(0.5);

    // Phase 1 — decode the visibility buffer and claim the sample.
    //
    // Resolving the material takes two dependent reads (`visible_slot` →
    // instance → `material_id`) and stops there: the full barycentric
    // reconstruction runs only for the pixels this dispatch owns, so a
    // scene with four materials does not reconstruct every pixel four
    // times.
    var mine = false;
    var surf: VertexOutput;
    var my_cell = vec3<u32>(0u);
    var my_slot = 0u;
    if (covered) {
        let visible_slot = u32(visibility) >> 7u;
        let inst_id = visible_meshlets[visible_slot] >> 16u;
        if (instances[inst_id].material_id == screen.material_id) {
            mine = true;
            my_slot = visible_slot;
            surf = resolve_surface(visible_slot, u32(visibility) & 0x7Fu, frag_coord);
            my_cell = inti_cluster_cell(surf.world_position, frag_coord);
            atomicMin(&tile_cell_min[0], my_cell.x);
            atomicMin(&tile_cell_min[1], my_cell.y);
            atomicMin(&tile_cell_min[2], my_cell.z);
            atomicMax(&tile_cell_max[0], my_cell.x);
            atomicMax(&tile_cell_max[1], my_cell.y);
            atomicMax(&tile_cell_max[2], my_cell.z);
        }
    }
    workgroupBarrier();

    // Phase 2 — cache the block's light lists.
    //
    // Every thread computes the same bounds off the same atomics, so
    // `cursor` below advances identically in all of them and the runs
    // need no atomic to be laid out. The loop bound is uniform across
    // the workgroup, which is what lets the barrier after it be reached
    // by everyone.
    let lo = vec3<u32>(
        atomicLoad(&tile_cell_min[0]),
        atomicLoad(&tile_cell_min[1]),
        atomicLoad(&tile_cell_min[2]));
    let hi = vec3<u32>(
        atomicLoad(&tile_cell_max[0]),
        atomicLoad(&tile_cell_max[1]),
        atomicLoad(&tile_cell_max[2]));
    // `lo.x > hi.x` means no thread claimed a pixel: the tile is all
    // background, all another material, or off screen.
    let empty = lo.x > hi.x;
    var dims = vec3<u32>(1u);
    var cell_count = 0u;
    if (!empty) {
        dims = hi - lo + vec3<u32>(1u);
        cell_count = dims.x * dims.y * dims.z;
    }
    let clustered = inti.clustered != 0u;
    // #826. Uniform across the workgroup, so the extra work in the load
    // loop below and the choice of walk in phase 3 are both scalar
    // branches rather than divergence.
    let sampling = inti.light_samples != 0u;
    // Cell count is checked before the loop; the light total is checked
    // inside it, because it is not known until the records are read.
    var cursor = 0u;
    if (clustered && cell_count > 0u && cell_count <= MAX_TILE_CELLS) {
        for (var c = 0u; c < cell_count; c = c + 1u) {
            let cell = lo + vec3<u32>(
                c / (dims.y * dims.z),
                (c / dims.z) % dims.y,
                c % dims.z);
            let rec = inti_clusters[inti_cluster_index(cell)];
            // Points and spots are consecutive ranges of the same
            // record, walked as one run — the grid stores them that way
            // precisely so no type test has to exist in the loop.
            //
            // Truncating against `cluster_capacity` reproduces the
            // fragment path's `break`: a frame whose lighting overflowed
            // the index list leaves later cells pointing past its end,
            // and both paths must stop at the same light or the images
            // differ where it matters least and is hardest to see.
            let end = min(rec.offset + rec.point_count + rec.spot_count, inti.cluster_capacity);
            let len = select(0u, end - rec.offset, end > rec.offset);
            if (cursor + len > MAX_TILE_LIGHTS) {
                if (lid == 0u) {
                    tile_overflow = 1u;
                }
                break;
            }
            if (lid == 0u) {
                tile_cell_start[c] = cursor;
                tile_cell_len[c] = len;
            }
            for (var i = lid; i < len; i = i + TILE_THREADS) {
                let light_index = inti_cluster_indices[rec.offset + i];
                tile_lights[cursor + i] = light_index;
                // #826 — the 80-byte record read ONCE PER TILE, here,
                // instead of once per pixel per light down in the walk.
                // The branch is uniform across the workgroup, and with
                // sampling off this stays exactly the #824 loop.
                if (sampling) {
                    let light = inti_lights[light_index];
                    // Luminance rather than the colour: the weight only
                    // has to rank the lights, and a saturated blue lamp
                    // and a white one of the same power should not rank
                    // differently because of the channel they land in.
                    // The 4π is `inti_sample_light`'s conversion from
                    // lumens to candela, applied here so the walk does
                    // not repeat it per pixel.
                    let luminance = dot(light.color, vec3<f32>(0.2126, 0.7152, 0.0722))
                        * light.intensity / (4.0 * INTI_PI);
                    tile_light_pos[cursor + i] = vec4<f32>(light.position, luminance);
                    tile_light_range[cursor + i] = light.range;
                    // 🔴 A light with a shadow map is never sampled.
                    // A shadow is a binary, high-contrast signal: losing
                    // a caster for a frame does not read as a slightly
                    // wrong estimate, it reads as a shadow that blinks.
                    // There are at most eight of them in the whole
                    // scene, so evaluating them all costs a bounded
                    // amount and removes the artefact entirely.
                    tile_light_caster[cursor + i] =
                        select(0u, 1u, light.shadow_slot != INTI_NO_SHADOW_SLOT);
                }
            }
            cursor = cursor + len;
        }
    } else if (!empty) {
        if (lid == 0u) {
            tile_overflow = 1u;
        }
    }
    workgroupBarrier();

    // Phase 2.5 — the tile chooses its lights (#826).
    //
    // Every condition guarding a barrier here is the same in all 256
    // threads — `sampling` and `clustered` come from the uniform,
    // `cell_count` from atomics all of them read, `tile_overflow` from
    // workgroup memory behind the barrier above. Uniform control flow is
    // not a style note in this function: a thread that skipped one of
    // these barriers would hang the ones that did not.
    if (sampling && clustered && tile_overflow == 0u && cell_count > 0u) {
        if (lid < cell_count) {
            atomicStore(&tile_rep_owner[lid], TILE_THREADS);
            atomicStore(&tile_pick_count[lid], 0u);
        }
        workgroupBarrier();

        // Elect one pixel per cell to speak for it.
        var my_c = 0u;
        if (mine) {
            let my = my_cell - lo;
            my_c = (my.x * dims.y + my.y) * dims.z + my.z;
            atomicMin(&tile_rep_owner[my_c], lid);
        }
        workgroupBarrier();

        // The elected thread publishes its surface point; meanwhile one
        // thread per cell puts that cell's shadow casters into the list,
        // before anything is sampled into it.
        if (mine && atomicLoad(&tile_rep_owner[my_c]) == lid) {
            tile_rep_pos[my_c] = surf.world_position;
        }
        if (lid < cell_count) {
            let start = tile_cell_start[lid];
            let len = tile_cell_len[lid];
            for (var i = 0u; i < len; i = i + 1u) {
                if (tile_light_caster[start + i] == 0u) {
                    continue;
                }
                let at = atomicAdd(&tile_pick_count[lid], 1u);
                if (at < MAX_TILE_PICKS) {
                    tile_pick[lid * MAX_TILE_PICKS + at] = start + i;
                    // Not chosen, so not scaled. A caster is evaluated
                    // because it is there, at its full contribution.
                    tile_pick_scale[lid * MAX_TILE_PICKS + at] = 1.0;
                }
            }
        }
        workgroupBarrier();

        // One thread per (cell, stratum) — at most 16 x 8 of the 256,
        // each walking its cell's run once. Against `(strata + 1)` walks
        // in every one of the 256 pixels, which is what the device
        // measured at 0.196 of an evaluation apiece.
        let strata = min(inti.light_samples, MAX_TILE_STRATA);
        if (lid < cell_count * strata) {
            let c = lid / strata;
            // A cell with no elected representative has no pixel in this
            // tile: it was reached by the block's bounding box and not by
            // any thread, so nothing will read what it chose.
            if (atomicLoad(&tile_rep_owner[c]) != TILE_THREADS) {
                let at = lo + vec3<u32>(
                    c / (dims.y * dims.z),
                    (c / dims.z) % dims.y,
                    c % dims.z);
                tile_choose(c, inti_cluster_index(at), lid % strata, strata);
            }
        }
        workgroupBarrier();
    }

    // Phase 3 — shade.
    if (mine) {
        let mat = materials[screen.material_id];
        let albedo = textureSampleGrad(
            albedo_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv);
        let base = albedo.rgb * mat.base_color.rgb;

        let n_ts = textureSampleGrad(
            normal_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv).xyz * 2.0 - 1.0;
        let n = normalize(surf.world_normal);
        let t = normalize(surf.world_tangent.xyz);
        let b = cross(n, t) * surf.world_tangent.w;
        let world_n = normalize(mat3x3<f32>(t, b, n) * n_ts);

        var rgb: vec3<f32>;
        // The debug views (#743). `inti_debug_is_view` is a literal
        // `false` in a production pipeline, so this branch and every
        // view behind it are gone before register allocation.
        if (inti_debug_is_view(screen.debug_mode)) {
            rgb = inti_debug_view(screen.debug_mode, surf.world_position, world_n, frag_coord);
        } else {
            let mr = textureSampleGrad(
                metal_rough_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv);
            let metallic = mat.metallic_roughness_emissive_pad.x * mr.b;
            let roughness = mat.metallic_roughness_emissive_pad.y * mr.g;

            var radiance: vec3<f32>;
            if (tile_overflow == 0u && clustered) {
                let my = my_cell - lo;
                let c = (my.x * dims.y + my.y) * dims.z + my.z;
                if (sampling) {
                    radiance = shade_picked_from_tile(
                        surf.world_position, world_n, base, metallic, roughness,
                        frag_coord, surf.flags, c);
                } else {
                    radiance = shade_from_tile(
                        surf.world_position, world_n, base, metallic, roughness,
                        frag_coord, surf.flags, tile_cell_start[c], tile_cell_len[c]);
                }
            } else {
                // The fallback the caps promise: straight to the storage
                // buffer, the same call the fragment path makes.
                radiance = inti_shade(
                    surf.world_position, world_n, base, metallic, roughness,
                    frag_coord, surf.flags);
            }
            radiance += base * mat.metallic_roughness_emissive_pad.z;
            rgb = inti_tonemap(radiance);
        }
        textureStore(color_out, vec2<i32>(sample), vec4<f32>(rgb, 1.0));
        // Only the upsample reads this, and only half rate has one. At
        // full rate the store would be one write per pixel bought
        // nothing — the branch is uniform across the workgroup, so it
        // costs a scalar compare.
        if (rate > 1u) {
            textureStore(shaded_ids, vec2<i32>(sample), vec4<u32>(my_slot + 1u, 0u, 0u, 0u));
        }
    }
}
