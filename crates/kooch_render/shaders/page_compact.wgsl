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
@group(0) @binding(3) var<storage, read_write> page_list: array<vec2<u32>>;
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
    page_list[listing] = vec2<u32>(page, stored - 1u);
    // The way back: a pass that computes a page KEY can now reach the
    // entry the draw indexes, without walking every resident page to
    // find it. See `PAGE_CELL`.
    table_slots[entry * PAGE_CELL + 2u] = listing;
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
    let meshlets = visible_counts[level];
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
}
