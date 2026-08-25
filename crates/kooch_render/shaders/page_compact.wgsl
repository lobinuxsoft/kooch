// page_compact.wgsl — the resident pages, as a list the GPU can dispatch
// over (#866).
//
// CONCATENATED after `page_table.wgsl`.
//
// # Why the flat table gets compacted at all
//
// The table is one entry per VIRTUAL page — ~half a million per view —
// and a frame makes a couple of thousand of them resident. Every pass
// after this one runs per page TIMES per meshlet, so walking the empty
// entries would multiply the emptiness by the scene's whole geometry.
// One pass over this view's span turns it into a dense list.
//
// # Bucketed by level, because the cull is
//
// A clipmap level is a texel density, and a density is a LOD. The
// meshlets that survive for level 3 are not the ones that survive for
// level 12, so the levels are culled separately and the pages have to
// be grouped the same way.
//
// 🔴 Local lights bucket by LIGHT, after the sun's levels: lamp
// `L`'s pages land in bucket `chain.x + L`, where its own cull's
// survivor list is bound. They used to bucket by octave into the SUN's
// buckets — borrowing survivor lists simplified for an orthographic box
// around the CAMERA, which broke both ways: a close lamp's casters fell
// outside the fine levels' box and its shadow vanished, and a coarse
// bucket handed root meshlets and drew spheres as faceted lumps. One
// bucket per lamp is the retired cube path's shape — one cull per
// light — not the feared bucket-per-light-per-LEVEL explosion: a lamp's
// pages of every level share its one perspective-LOD survivor list,
// because a perspective error metric already scales with distance.

@group(0) @binding(0) var<uniform> raster: PageRaster;
// The flat table: `PAGE_CELL` words per virtual page — `slot + 1`
// (`PAGE_ABSENT` = not resident), the age, the listing. It is the
// marking pass's buffer and this reads it with the same stride or it
// reads an age as a slot.
@group(0) @binding(2) var<storage, read_write> table_slots: array<u32>;
// `x` the virtual page, `y` its physical slot. Bucketed: level `L`
// owns `[L * chain.z, (L + 1) * chain.z)`.
// Four words per listing: the page, its slot, the furthest receiver
// this frame recorded on it (#940, f32 bits, 0 = none), and a spare.
// Widened rather than given a sibling buffer: the expansion sits AT
// the eight-storage-buffer downlevel limit and cannot bind the table.
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
// One generation per bucket owner, per view: the sun's levels first
// (snapped centre + direction), then the lamps (transform, range,
// cone). A page whose stamp equals its owner's generation keeps last
// frame's content and is never listed. Never zero.
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
    // One thread per entry of THIS VIEW'S span. The table is flat and a
    // view's entries are a contiguous run, so the other cameras' pages
    // are outside the dispatch rather than a decode-and-skip — which is
    // also why the "belongs to another view" counter is gone.
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
        // An OCTAVE of world texel size, anchored so the clipmap's
        // level L lands on bucket L exactly — the level whose cull was
        // handed that density. Virtual TEXELS across level 0: pages
        // across it times a page's side; `space.y` is a face's page
        // count and would be off by the page size squared.
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
    // The cache gate: a page whose content was drawn under the
    // generation its owner still has keeps it — not listed, not
    // stamped, not drawn. `page_stamp` zeroes fresh claims and
    // `cs_invalidate` zeroes touched pages, so 0 never matches.
    var gen_at = id.level;
    if !id.is_sun {
        gen_at = sun_buckets + id.light;
    }
    var gen = gens[raster.views.x * buckets + gen_at];
    // 🔴 A sun page's validity is PER PAGE, not per level. `sun_cell`
    // keys by absolute world position and wraps into the table, so when
    // the window scrolls the ring that enters lands on the very slots
    // the ring that left was using: same key, different piece of world.
    // Folding the page's own absolute index into the generation is what
    // tells those two apart — the interior, whose index did not move,
    // matches and keeps its content; the wrapped ring does not and
    // redraws. The level-wide generation carries only what is genuinely
    // level-wide (the sun's direction, the depth anchor, the scene).
    if id.is_sun {
        let basis = sun_basis(raster.sun.xyz);
        let idx = sun_page_index(
            id.level, id.cell, raster.eye.xyz, basis, raster.world.x, raster.space.z);
        gen = page_mix(gen, bitcast<u32>(i32(idx.x)));
        gen = page_mix(gen, bitcast<u32>(i32(idx.y)));
        // 0 means "no content" and must never match a generation.
        gen = gen | 1u;
    }
    if table_slots[entry * PAGE_CELL + 3u] == gen {
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
        table_slots[entry * PAGE_CELL + 4u],
        0u,
    );
    // The way back: a pass that computes a page KEY can now reach the
    // entry the draw indexes, without walking every resident page to
    // find it. See `PAGE_CELL`.
    table_slots[entry * PAGE_CELL + 2u] = listing;
    // Listed means "drawn this frame", which is when the content
    // becomes this generation's. Stamped here rather than after the
    // draw because nothing between the two can fail — the one thing
    // that can, a pair-list overflow, is counted and handled by the
    // CPU bumping the scene generation.
    table_slots[entry * PAGE_CELL + 3u] = gen;
    let d = atomicAdd(&dirty[0], 1u);
    if d + 1u < arrayLength(&dirty) {
        atomicStore(&dirty[1u + d], stored - 1u);
    }
}

// Zeroes the content stamp of every page a moved caster can reach —
// the shadow it cast (old bounds) and the one it casts now (new
// bounds) both have to redraw. One thread per entry of THIS VIEW's
// span, a loop over the handful of moved spheres inside it.
//
// A lamp page invalidates at LIGHT granularity — sphere against the
// light's range — which over-invalidates that lamp's few pages and
// never misses; the sun's pages, where the volume is, test their own
// rect. Per-cell lamp tests are #866's refinement.
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
    let threads = pages * meshlets;
    expand_args[level * 3u + 0u] = (threads + EXPAND_GROUP - 1u) / EXPAND_GROUP;
    expand_args[level * 3u + 1u] = 1u;
    expand_args[level * 3u + 2u] = 1u;
}

// One thread, after every level has expanded: the draw covers all of
// them at once, so its instance count is the whole pair list.
//
// 🔴 ONE `draw_indirect` for the entire clipmap, not one per level. A
// pair carries the packed `(instance, meshlet)` its cull produced, which
// is self-describing, so the draw never has to know which level a pair
// came from.
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
