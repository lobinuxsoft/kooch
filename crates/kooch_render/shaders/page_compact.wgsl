// page_compact.wgsl — the resident pages, as a list the GPU can dispatch over (#866).

@group(0) @binding(0) var<uniform> raster: PageRaster;
// The flat table: `PAGE_CELL` words per virtual page — `slot + 1` (`PAGE_ABSENT` = not resident),
// the age, the listing. It is the marking pass's buffer and this reads it with the same stride or
// it reads an age as a slot.
@group(0) @binding(2) var<storage, read_write> table_slots: array<u32>;
// `x` the virtual page, `y` its physical slot. Bucketed: level `L` owns `[L * chain.z, (L + 1) *
// chain.z)`. Four words per listing: the page, its slot, the furthest receiver this frame recorded
// on it (#940, f32 bits, 0 = none), and a spare.
@group(0) @binding(3) var<storage, read_write> page_list: array<vec4<u32>>;
// x..levels the pages listed per level, then: the sun pages that did
// not fit a bucket, the local-light pages skipped, the pairs, the pairs
// that overflowed, and the pages belonging to ANOTHER view.
@group(0) @binding(4) var<storage, read_write> page_counts: array<atomic<u32>>;
// One `dispatch_workgroups_indirect` argument triple per level.
@group(0) @binding(5) var<storage, read_write> expand_args: array<u32>;
// The visible meshlet count each level's cull produced, copied out of
// its draw arguments.
@group(0) @binding(6) var<storage, read> visible_counts: array<u32>;
@group(0) @binding(7) var<storage, read_write> draw_args: array<u32>;
// One generation per bucket owner, per view: the sun's levels first (snapped centre + direction),
// then the lamps (transform, range, cone). A page whose stamp equals its owner's generation keeps
// last frame's content and is never listed. Never zero.
@group(0) @binding(8) var<storage, read> gens: array<u32>;
// `[0]` the count, then the physical SLOT of every page listed this
// dispatch — what the depth pass clears, page by page, now that whole
// layers are never wiped.
@group(0) @binding(9) var<storage, read_write> dirty: array<atomic<u32>>;
// `[0].x` the count as f32, then world spheres — old and new bounds of
// every caster that moved this frame. Only `cs_invalidate` reads them.
@group(0) @binding(10) var<storage, read> moved: array<vec4<f32>>;
// Only `cs_invalidate` reads them, for a lamp page's range test.
@group(0) @binding(11) var<storage, read> inv_lights: array<ClusterLight>;
const COMPACT_GROUP: u32 = 64u;
const EXPAND_GROUP: u32 = 64u;

@compute @workgroup_size(COMPACT_GROUP, 1, 1)
fn cs_compact(@builtin(global_invocation_id) gid: vec3<u32>) {
    // One thread per entry of THIS VIEW'S span. The table is flat and a view's entries are a
    // contiguous run, so the other cameras' pages are outside the dispatch rather than a
    // decode-and-skip — which is also why the "belongs to another view" counter is gone.
    if gid.x >= raster.views.y {
        return;
    }
    let entry = raster.views.x * raster.views.y + gid.x;
    let stored = table_slots[entry * PAGE_CELL];
    if stored == PAGE_ABSENT {
        return;
    }
    // Whatever listing this entry carried belongs to a compaction that
    // is over.
    table_slots[entry * PAGE_CELL + 2u] = PAGE_UNLISTED;
    // The entry index IS the virtual page id.
    let page = entry;
    let id = page_decode(
        page,
        raster.views.y,
        raster.space.x,
        raster.space.y,
        raster.space.z,
        raster.space.w,
        raster.pool.w,
    );
    // Sun buckets first, then one bucket per lamp — the counters sit
    // after ALL of them, which is what `buckets` indexes here.
    let sun_buckets = raster.chain.x;
    let buckets = sun_buckets + LAMP_CULLS;
    var slot: u32;
    if id.is_sun {
        // An OCTAVE of world texel size, anchored so the clipmap's level L lands on bucket L
        // exactly — the level whose cull was handed that density.
        let virtual_texels = raster.space.z * raster.pool.w;
        let texel = page_texel_world(id, raster.world.x, virtual_texels, 0.0);
        slot = page_octave(texel, raster.world.x, virtual_texels, sun_buckets);
    } else {
        // The lamp's OWN bucket, where its own cull's survivors are
        // bound. A light past the cull budget stays undrawn — counted
        // with the dropped pages, not silent.
        if id.light >= LAMP_CULLS {
            atomicAdd(&page_counts[buckets], 1u);
            return;
        }
        slot = sun_buckets + id.light;
    }
    // The cache gate: a page whose content was drawn under the generation its owner still has keeps
    // it — not listed, not stamped, not drawn. `page_stamp` zeroes fresh claims and `cs_invalidate`
    // zeroes touched pages, so 0 never matches.
    var gen_at = id.level;
    if !id.is_sun {
        gen_at = sun_buckets + id.light;
    }
    var gen = gens[raster.views.x * buckets + gen_at];
    // 🔴 A sun page's validity is PER PAGE, not per level. `sun_cell` keys by absolute world
    // position and wraps into the table, so when the window scrolls the ring that enters lands on
    // the very slots the ring that left was using: same key, different piece of world.
    if id.is_sun {
        let basis = sun_basis(raster.sun.xyz);
        let idx = sun_page_index(
            id.level, id.cell, raster.eye.xyz, basis, raster.world.x, raster.space.z);
        gen = page_mix(gen, bitcast<u32>(i32(idx.x)));
        gen = page_mix(gen, bitcast<u32>(i32(idx.y)));
        // 0 means "no content" and must never match a generation.
        gen = gen | 1u;
    }
    let stamp = table_slots[entry * PAGE_CELL + 3u];
    if stamp == gen {
        atomicAdd(&page_counts[buckets + 4u], 1u);
        return;
    }
    // 🔴 An EMPTY page is valid under every generation, and that is what rescues a moving light
    // (#1022).
    let survivors = visible_counts[slot];
    if !id.is_sun && stamp == PAGE_EMPTY && survivors == 0u {
        atomicAdd(&page_counts[buckets + 4u], 1u);
        return;
    }
    // Local pages are still counted separately, because "listed" and
    // "drawn" are different claims and the panel states both.
    if !id.is_sun {
        atomicAdd(&page_counts[buckets + 1u], 1u);
    }
    let index = atomicAdd(&page_counts[slot], 1u);
    if index >= raster.chain.z {
        // The bucket is full. Undoing the add would race; the count is
        // left high on purpose so the overflow is visible rather than
        // silently clamped.
        atomicAdd(&page_counts[buckets], 1u);
        return;
    }
    let listing = slot * raster.chain.z + index;
    page_list[listing] = vec4<u32>(
        page,
        stored - 1u,
        // The third word is retired with Olsson's receiver bound; the
        // slot it used now holds `PAGE_LOD`, which the draw never reads.
        0u,
        0u,
    );
    // The way back: a pass that computes a page KEY can now reach the
    // entry the draw indexes, without walking every resident page to
    // find it. See `PAGE_CELL`.
    table_slots[entry * PAGE_CELL + 2u] = listing;
    // Listed means "drawn this frame", which is when the content becomes this generation's. Stamped
    // here rather than after the draw because nothing between the two can fail — the one thing that
    // can, a pair-list overflow, is counted and handled by the CPU bumping the scene generation.
    table_slots[entry * PAGE_CELL + 3u] = select(gen, PAGE_EMPTY, !id.is_sun && survivors == 0u);
    let d = atomicAdd(&dirty[0], 1u);
    if d + 1u < arrayLength(&dirty) {
        atomicStore(&dirty[1u + d], stored - 1u);
    }
}

// Zeroes the content stamp of every page a moved caster can reach — the shadow it cast (old bounds)
// and the one it casts now (new bounds) both have to redraw. One thread per entry of THIS VIEW's
// span, a loop over the handful of moved spheres inside it.
@compute @workgroup_size(COMPACT_GROUP, 1, 1)
fn cs_invalidate(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= raster.views.y {
        return;
    }
    let entry = raster.views.x * raster.views.y + gid.x;
    if table_slots[entry * PAGE_CELL] == PAGE_ABSENT {
        return;
    }
    if table_slots[entry * PAGE_CELL + 3u] == 0u {
        return;
    }
    let count = u32(moved[0].x);
    if count == 0u {
        return;
    }
    let id = page_decode(
        entry,
        raster.views.y,
        raster.space.x,
        raster.space.y,
        raster.space.z,
        raster.space.w,
        raster.pool.w,
    );
    for (var i = 0u; i < count; i = i + 1u) {
        let sphere = moved[1u + i];
        var hit = false;
        if id.is_sun {
            let basis = sun_basis(raster.sun.xyz);
            let centre =
                sun_centre(raster.eye.xyz, basis, raster.world.x, raster.space.z, id.level);
            let rect = sun_page_rect(id.level, id.cell, raster.eye.xyz, basis, raster.world.x, raster.space.z);
            let plane = sun_plane(sphere.xyz, basis);
            let along = dot(sphere.xyz - raster.eye.xyz, basis[2])
                + sun_drift(raster.eye.xyz, basis, raster.world.x, raster.space.z, id.level);
            let half = rect.z * 0.5 + sphere.w;
            hit = abs(plane.x - rect.x) <= half
                && abs(plane.y - rect.y) <= half
                && abs(along) <= raster.world.y + sphere.w;
        } else if id.light < arrayLength(&inv_lights) {
            let light = inv_lights[id.light];
            hit = distance(sphere.xyz, light.position) <= sphere.w + light.range;
        }
        if hit {
            table_slots[entry * PAGE_CELL + 3u] = 0u;
            return;
        }
    }
}

// One thread per level: the expansion's dispatch size is pages TIMES
// meshlets, and both numbers only exist on the GPU.
@compute @workgroup_size(EXPAND_GROUP, 1, 1)
fn cs_expand_args(@builtin(global_invocation_id) gid: vec3<u32>) {
    let level = gid.x;
    if level >= raster.chain.x + LAMP_CULLS {
        return;
    }
    let pages = min(atomicLoad(&page_counts[level]), raster.chain.z);
    var meshlets = visible_counts[level];
    // A lamp's count is written uncapped so its overflow is visible;
    // the dispatch is sized to the slice that actually exists.
    if level >= raster.chain.x {
        meshlets = min(meshlets, LAMP_SURVIVORS);
    }
    // 🔴 The inverted expansion runs ONE thread per survivor, and only for the sun's buckets — the
    // pyramid it descends covers one clipmap. Sizing this the paired way would run the descent once
    // per page and every pair would land `pages` times.
    let inverted = raster.layer.w != 0u && level < raster.chain.x;
    let threads = select(pages * meshlets, meshlets, inverted);
    expand_args[level * 3u + 0u] = (threads + EXPAND_GROUP - 1u) / EXPAND_GROUP;
    expand_args[level * 3u + 1u] = 1u;
    expand_args[level * 3u + 2u] = 1u;
}

// One thread, after every level has expanded: the draw covers all of them at once, so its instance
// count is the whole pair list.
@compute @workgroup_size(1, 1, 1)
fn cs_draw_args() {
    let pairs = min(atomicLoad(&page_counts[raster.chain.x + LAMP_CULLS + 2u]), raster.chain.y);
    // Vertex count is fixed per meshlet: the draw is indirect over a
    // triangle budget and the tail of a shorter meshlet is discarded in
    // the vertex shader.
    draw_args[0] = raster.chain.w * 3u;
    draw_args[1] = pairs;
    draw_args[2] = 0u;
    draw_args[3] = 0u;
    // The second draw: one quad per dirty page, wiping exactly the
    // rects the pairs are about to fill — the whole-layer clear died
    // with the cache.
    draw_args[4] = 4u;
    draw_args[5] = min(atomicLoad(&dirty[0]), arrayLength(&dirty) - 1u);
    draw_args[6] = 0u;
    draw_args[7] = 0u;
}

/// Fills `PAGE_LOD` for every page of the sun's clipmap: how many levels up the first READABLE page
/// covering the same world position sits.
@compute @workgroup_size(64, 1, 1)
fn cs_lod_offsets(@builtin(global_invocation_id) gid: vec3<u32>) {
    let side = raster.space.z;
    let levels = raster.chain.x;
    let per_level = side * side;
    if gid.x >= levels * per_level {
        return;
    }
    let level = gid.x / per_level;
    let within = gid.x % per_level;
    let cell = vec2<u32>(within % side, within / side);

    let base = raster.views.x * raster.views.y + raster.space.w * raster.space.x;
    let entry = base + level * per_level + cell.y * side + cell.x;

    let basis = sun_basis(raster.sun.xyz);
    let eye = raster.eye.xyz;
    // This page's ABSOLUTE index on the level's world grid — the one
    // identity that survives the wrap.
    let absolute = sun_page_index(level, cell, eye, basis, raster.world.x, side);

    var found = PAGE_NO_LOD;
    for (var step = 0u; level + step < levels; step = step + 1u) {
        let up = level + step;
        // `>> step` on the absolute index, then wrapped into the coarser
        // level's window the way `sun_cell` keys it.
        let scaled = floor(absolute / exp2(f32(step)));
        let coarse = vec2<u32>(wrap_to(scaled, f32(side)));
        let at = base + up * per_level + coarse.y * side + coarse.x;
        // Readable, not merely resident: a page with no content stamp
        // holds a clear, and a clear reads as "nothing occludes".
        if table_slots[at * PAGE_CELL] != PAGE_ABSENT
            && table_slots[at * PAGE_CELL + 3u] != 0u
        {
            found = step;
            break;
        }
    }
    table_slots[entry * PAGE_CELL + PAGE_LOD] = found;
}
