// page_compact.wgsl — the resident pages, as a list the GPU can dispatch
// over (#866).
//
// CONCATENATED after `page_table.wgsl`.
//
// # Why the hash gets compacted at all
//
// The table is 8192 entries for a 4096-page pool and a frame makes a
// couple of thousand of them resident. Every pass after this one runs
// per page TIMES per meshlet, so walking the empty entries would
// multiply the emptiness by the scene's whole geometry. One 8192-thread
// pass turns it into a dense list.
//
// # Bucketed by level, because the cull is
//
// A clipmap level is a texel density, and a density is a LOD. The
// meshlets that survive for level 3 are not the ones that survive for
// level 12, so the levels are culled separately and the pages have to
// be grouped the same way.
//
// 🔴 Local lights are counted and skipped. Their pages are marked and
// allocated — that half works — but rasterising them needs a cull per
// (light, face, level), which is 4848 views against the sun's 17. That
// is a different machine and it is the next one.

@group(0) @binding(0) var<uniform> raster: PageRaster;
@group(0) @binding(1) var<storage, read> table_keys: array<u32>;
// TWO words an entry — the slot, then the frame it was last requested
// in. See `PAGE_CELL`: it is the marking pass's buffer and this reads it
// with the same stride or it reads an age as a slot.
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
    let entry = gid.x;
    if entry >= raster.pool.x {
        return;
    }
    let key = table_keys[entry];
    // 🔴 DEAD as well as EMPTY. The pool persists, so eviction leaves a
    // tombstone rather than an empty entry — and `PAGE_DEAD - 1` decodes
    // into a perfectly well-formed view, light, level and cell, none of
    // which mean anything. Skipping only EMPTY rasterises that garbage
    // into a slot nobody owns, which is what a scene full of squares in
    // the wrong places looks like.
    if key == PAGE_EMPTY || key == PAGE_DEAD {
        return;
    }
    // Whatever listing this entry carried belongs to a compaction that
    // is over. Cleared before the reasons to return below, so a page
    // this view does not own never keeps another view's index.
    table_slots[entry * PAGE_CELL + 2u] = PAGE_UNLISTED;
    // Keys are stored as `page + 1` so that a cleared buffer is empty.
    let page = key - 1u;
    let id = page_decode(
        page,
        raster.views.y,
        raster.space.x,
        raster.space.y,
        raster.space.z,
        raster.space.w,
    );
    let levels = raster.chain.x;
    // 🔴 Not a cap and not a failure: the table holds every view's
    // pages, and each view compacts its own. Counted anyway, because
    // "the pool is full and my view got forty pages" is unreadable
    // without knowing how many belong to somebody else — which is the
    // exact number that would have named the last defect on sight.
    if id.view != raster.views.x {
        atomicAdd(&page_counts[levels + 4u], 1u);
        return;
    }
    if !id.is_sun {
        atomicAdd(&page_counts[levels + 1u], 1u);
        return;
    }
    if id.level >= levels {
        atomicAdd(&page_counts[levels], 1u);
        return;
    }
    let index = atomicAdd(&page_counts[id.level], 1u);
    if index >= raster.chain.z {
        // The bucket is full. Undoing the add would race; the count is
        // left high on purpose so the overflow is visible rather than
        // silently clamped.
        atomicAdd(&page_counts[levels], 1u);
        return;
    }
    let listing = id.level * raster.chain.z + index;
    page_list[listing] = vec2<u32>(page, table_slots[entry * PAGE_CELL]);
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
    if level >= raster.chain.x {
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
    let levels = raster.chain.x;
    let pairs = min(atomicLoad(&page_counts[levels + 2u]), raster.chain.y);
    // Vertex count is fixed per meshlet: the draw is indirect over a
    // triangle budget and the tail of a shorter meshlet is discarded in
    // the vertex shader.
    draw_args[0] = raster.chain.w * 3u;
    draw_args[1] = pairs;
    draw_args[2] = 0u;
    draw_args[3] = 0u;
}
