// page_mark.wgsl — which shadow pages a frame actually needs (#866).
//
// CONCATENATED after `cluster_common.wgsl`, the way the grid's own
// passes are built, so `ClusterView`, `ClusterCell`, `ClusterLight` and
// the two cell lookups are the grid's declarations rather than a second
// copy free to drift from them.
//
// # What marks a page, and why it is not the grid alone
//
// One thread per screen pixel — or per block of them, see `rate`. The
// depth buffer says WHERE a surface is; the froxel grid says WHICH
// lights reach it. Both are needed and neither is sufficient:
//
// - Marking from the grid's cells alone claims pages for ground no
//   surface occupies. Measured on `many_lights.scene`, that is 15770
//   pages for the sun where the surfaces need 118 — 133x.
// - Marking from depth alone would have to walk every light per pixel,
//   which is the loop the grid exists to remove.
//
// Epic states the first half in one sentence — "depth buffer analysis is
// used as the primary method of marking pages that are needed to render"
// — and the Chalmers papers state the second: the page allocation is
// driven by the cluster's view samples.
//
// # The mirror
//
// Every arithmetic decision below has a twin in `shadow/pages.rs` on the
// CPU: `page_level`/`level_for`, `sun_level`/`level_above`+`level_below`,
// `page_of`/`page_rect`. That is deliberate — the CPU census is this
// pass's oracle, and two counts that disagree mean one of them is wrong.

// Mirrors `PageMarkView` in `pages/mark.rs`, field for field.
struct PageView {
    world_from_clip: mat4x4<f32>,
    // xyz the camera, w the clipmap's level-0 extent in metres.
    eye_and_base: vec4<f32>,
    // xyz the sun's direction, w 1 if there is one.
    sun: vec4<f32>,
    // x page texels, y virtual texels, z levels in a local chain,
    // w levels in the clipmap.
    chain: vec4<u32>,
    // x pages per side at level 0, y pages in one face's whole chain,
    // z the per-light stride in pages, w the light count.
    //
    // 🔴 `z` is a multiple of 32 by construction. The mark bitmap is
    // emptied one VIEW at a time and a view's bits have to start on a
    // word boundary — `clear_buffer` takes byte offsets, so a stride
    // that put a view's first bit mid-word would clear the neighbour's.
    strides: vec4<u32>,
    // x the sampling rate in pixels, y the sun's slot WITHIN a view,
    // z 1 when the debug view is painting, w which view this is.
    sampling: vec4<u32>,
    // x entries in the page table, y physical pages the pool holds,
    // z pages across the atlas, w the pool slots ONE VIEW owns.
    pool: vec4<u32>,
    // xy how many output pixels one depth pixel covers, zw the output
    // size.
    //
    // 🔴 The paint target is the view's FINAL colour buffer, which is
    // allocated at the output size while the depth is at the render
    // size. They differ whenever `render_scale` is below 100, and one
    // thread then owns a block rather than a pixel.
    paint: vec4<f32>,
    // x THIS FRAME's index, y how many frames a page may go unrequested
    // before it is evicted, z 1 when the pool is being rebuilt from
    // nothing, w unused.
    //
    // 🔴 The frame index is what makes the pool persistent. A page is
    // not freed because a frame ended; it is freed because `max_age`
    // frames passed without anything asking for it. That is Epic's
    // `MaxPageAgeSinceLastRequest` and it is the whole mechanism — a
    // page that survives is a page nothing has to rasterise again.
    life: vec4<u32>,
    // x the RECIPROCAL of `shadow_density`, as a fraction of 1.
    //
    // 🔴 The one lever the census found. It multiplies the world size a
    // screen pixel is allowed to ask a shadow texel to match, so a
    // density of 50 % doubles `wanted`, which is one level coarser in
    // BOTH axes — a quarter of the pages.
    density: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: ClusterView;
// The read side of `ClusterCell`, without the atomics.
//
// A read-only storage binding cannot hold `atomic<u32>`, and the grid
// declares its cells atomic because two rasterizer passes write them
// from many fragments at once. `inti_pbr.wgsl` mirrors the same record
// as `IntiClusterCell` for the same reason; this is the third reader and
// the layout is pinned by the same test.
struct PageCell {
    offset: u32,
    point_count: u32,
    spot_count: u32,
    probe_count: u32,
    volume_count: u32,
    decal_count: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(1) var<storage, read> cells: array<PageCell>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read> lights: array<ClusterLight>;
@group(0) @binding(4) var depth_tex: texture_depth_2d;
@group(0) @binding(5) var<uniform> pages: PageView;
@group(0) @binding(6) var<storage, read_write> marks: array<atomic<u32>>;
// x the distinct pages marked, y the samples that found a surface,
// z pairs visited, w overflow — a page index past the buffer.
@group(0) @binding(7) var<storage, read_write> counters: array<atomic<u32>>;
// The frame's FINAL colour, overwritten where the debug view paints.
//
// 🔴 Not the HDR radiance target: that one lives inside the R64 stage
// and this pass cannot reach it. Painting the tonemapped image instead
// means no exposure to divide out and nothing downstream to survive.
//
// ⚠️ `rgba8unorm` has to match `DEFERRED_COLOR_FORMAT` exactly. wgpu
// compares the storage class declared here against the bind group
// layout, and a mismatch surfaces as "Storage texture binding 8 expects
// format ..." rather than as a wrong image.
@group(0) @binding(8) var color_out: texture_storage_2d<rgba8unorm, write>;

// The page table, FLAT: the entry index IS the virtual page id, so the
// lookup the shading pass runs per pixel per light is one load.
// `page_table.wgsl` holds the id arithmetic and the atlas layout —
// everything the READER has to agree with, kept in one file so the two
// cannot drift.
//
// `PAGE_CELL` words per entry: `slot + 1` (`PAGE_ABSENT` = no page, so
// a cleared buffer is an empty table), the frame it was last REQUESTED
// in, and its index in this view's compacted `page_list`. Interleaved
// rather than three buffers because
// `max_storage_buffers_per_shader_stage` is EIGHT on the downlevel
// defaults and this pass was already there — and a hit reads the slot
// and writes the age, so they share a cache line anyway.
@group(0) @binding(10) var<storage, read_write> table_cells: array<atomic<u32>>;
// The allocator's own state, laid out per view: `[high, free_count,
// free_slots...]` repeated every `slice + 2` words.
//
// 🔴 NOT cleared between frames, which is the difference between this
// and what came before. The bump high-water mark and the free list are
// what a page's residency survives on; a `clear_buffer` over them every
// frame is exactly the non-persistent pool this replaces.
@group(0) @binding(11) var<storage, read_write> alloc: array<atomic<u32>>;

// The words of a table entry. `PAGE_CELL` is in `page_table.wgsl`
// because the READER indexes the same buffer. The first word stores
// `slot + 1`; this unwraps it and is only called on a resident entry.
fn page_slot(entry: u32) -> u32 {
    return atomicLoad(&table_cells[entry * PAGE_CELL]) - 1u;
}

fn page_age(entry: u32) -> u32 {
    return atomicLoad(&table_cells[entry * PAGE_CELL + 1u]);
}

fn page_stamp(entry: u32, slot: u32, frame: u32) {
    atomicStore(&table_cells[entry * PAGE_CELL], slot + 1u);
    atomicStore(&table_cells[entry * PAGE_CELL + 1u], frame);
}

fn page_refresh(entry: u32, frame: u32) {
    atomicStore(&table_cells[entry * PAGE_CELL + 1u], frame);
}

const NO_PAGE: u32 = 0xffffffffu;

const MARK_GROUP: u32 = 8u;

// The pages one view addresses, and where its own start.
//
// The pages one view addresses: `sun_slot` locals at `strides.z` pages
// each, then the sun's clipmap — `chain.w` levels of a full grid.
// Derived rather than uploaded: a second copy of it in the uniform is a
// second thing to keep in step with `strides.z`.
fn view_span() -> u32 {
    let raw = pages.sampling.y * pages.strides.z
        + pages.chain.w * pages.strides.x * pages.strides.x;
    // On a word boundary, mirroring `span` on the CPU: view N's bits
    // start at `N * span` and the bitmap is cleared per view.
    return (raw + 31u) / 32u * 32u;
}

fn view_base() -> u32 {
    return pages.sampling.w * view_span();
}

// Where this view's allocator state starts in `alloc`.
//
// `[high, free_count, free_slots...]`, one run per view, so a camera
// allocates and frees without touching another camera's list.
fn alloc_base() -> u32 {
    return pages.sampling.w * (pages.pool.w + 2u);
}

// A physical slot out of this view's slice, recycled first.
//
// 🔴 Free list BEFORE the bump, and that ordering is the point of the
// whole change. A pool that only ever bumps runs out after `slice`
// distinct pages have EVER been requested — which, with a camera that
// moves, is a matter of seconds. Recycling is what turns "pages this
// session needed" into "pages this moment needs".
fn page_alloc() -> u32 {
    let base = alloc_base();
    let slice = pages.pool.w;

    // Pop. `atomicSub` returning the OLD value is what makes the test
    // and the take one operation: a thread that sees a count of zero or
    // less pushed it below zero and puts it back.
    let taken = atomicSub(&alloc[base + 1u], 1u);
    if taken != 0u && taken <= slice {
        return atomicLoad(&alloc[base + 2u + taken - 1u]);
    }
    atomicAdd(&alloc[base + 1u], 1u);

    let local = atomicAdd(&alloc[base], 1u);
    if local >= slice {
        atomicSub(&alloc[base], 1u);
        atomicAdd(&counters[5], 1u);
        return PAGE_MISS;
    }
    return pages.sampling.w * slice + local;
}

// Gives a slot back to this view's free list.
fn page_release(slot: u32) {
    let base = alloc_base();
    let slice = pages.pool.w;
    let at = atomicAdd(&alloc[base + 1u], 1u);
    if at >= slice {
        // The list cannot hold more than the slice does, so this cannot
        // happen without the slice having been double-freed. Undo and
        // leak the slot rather than write past the run.
        atomicSub(&alloc[base + 1u], 1u);
        atomicAdd(&counters[10], 1u);
        return;
    }
    atomicStore(&alloc[base + 2u + at], slot);
}

// Finds `page` in the table, or puts it there, and stamps it with this
// frame either way.
//
// 🔴 The common case is the first branch: the page is already resident,
// its age is refreshed, and NOTHING is allocated and nothing has to be
// rasterised again. `counters[7]` counts those and `counters[8]` counts
// the ones that really are new — the two together are what says whether
// persistence is doing anything.
//
// No compare-exchange. Only the thread that flipped the page's mark bit
// calls this, so the entry is this thread's own — and the eviction runs
// in an earlier dispatch of the same pass, which is a barrier.
fn page_touch(page: u32) -> u32 {
    if page >= pages.pool.x {
        return PAGE_MISS;
    }
    let frame = pages.life.x;
    let stored = atomicLoad(&table_cells[page * PAGE_CELL]);
    if stored != PAGE_ABSENT {
        page_refresh(page, frame);
        atomicAdd(&counters[7], 1u);
        return stored - 1u;
    }
    let slot = page_alloc();
    if slot == PAGE_MISS {
        return PAGE_MISS;
    }
    page_stamp(page, slot, frame);
    atomicAdd(&counters[8], 1u);
    return slot;
}

// One bit, set once. The return says whether this thread is the one that
// set it, which is what makes the counter a count of DISTINCT pages
// rather than of marking attempts — and what makes it the right place
// to allocate from.
//
// 🔴 `claim` is what separates a page the frame NEEDS from a page the
// frame can USE. Marking a local light's page is a measurement — this
// track's whole justification is what a hundred casting lights would
// cost — but the raster only draws the sun, so a slot handed to a local
// page is a slot nothing ever writes and nothing ever samples.
//
// Measured before this split, on `many_lights` with two viewports:
// **991 and 1004 of each camera's 1024 slots were local**, leaving the
// sun 33 and 20 pages. The scene had almost no shadow, and the pool
// reported itself 100 % full while doing nothing.
//
// Epic states the same rule as a pass: `PruneLightGridCS` rewrites the
// light grid down to the lights that HAVE a virtual shadow map before
// anything marks. The gate moves here the day the local raster lands.
fn mark_bit(index: u32, claim: bool) -> bool {
    let word = index / 32u;
    if word >= arrayLength(&marks) {
        atomicAdd(&counters[3], 1u);
        return false;
    }
    let bit = 1u << (index % 32u);
    let was = atomicOr(&marks[word], bit);
    if (was & bit) != 0u {
        return false;
    }
    atomicAdd(&counters[0], 1u);
    if claim {
        // WGSL has no call statement for a function that returns; the
        // slot is the sampling pass's business, not this one's.
        _ = page_touch(index);
    }
    return true;
}

// Where `level` starts inside one face's chain, measured from the
// FLOOR: the levels under `local_level_floor` are not addressed at all.
// See `local_level_base` for why the offset is a running sum.
fn level_base(level: u32) -> u32 {
    return local_level_base(level, pages.strides.x, pages.chain.x);
}

fn level_side(level: u32) -> u32 {
    return max(pages.strides.x >> level, 1u);
}

// The coarsest level whose texels are still at least as dense as the
// screen's pixels. A cube face spans 90 degrees, so at `distance` it
// covers `2 * distance` world units across its texels.
//
// 🔴 FLOORED at `local_level_floor`. A pixel next to a lamp asks for a
// texel the lamp has no business providing, and the pool pays for every
// page of it — see `LOCAL_MAX_TEXELS` for what that measured.
fn page_level(distance: f32, wanted: f32) -> u32 {
    let base = local_level_floor(pages.chain.y);
    if wanted <= 0.0 {
        return base;
    }
    let texels = 2.0 * distance / wanted;
    if texels <= 0.0 {
        return pages.chain.z - 1u;
    }
    let level = floor(log2(f32(pages.chain.y) / texels));
    return clamp(u32(max(level, 0.0)), base, pages.chain.z - 1u);
}

// One page of a local light's mip chain.
// Which page of a local light's chain a point belongs to, WITHOUT
// marking it. Split for the same reason as `sun_page_for`.
fn local_page_for(light: u32, world: vec3<f32>, wanted: f32) -> vec2<u32> {
    let record = lights[light];
    var offset = world - record.position;
    let distance = max(length(offset), 0.05);
    let level = page_level(distance, wanted);
    let side = level_side(level);

    // A spot's one face is aligned with ITS axis, not the world's —
    // see `spot_local` for the three-way disagreement this rotation
    // ended.
    let spot = record.kind == PAGE_KIND_SPOT;
    if spot {
        offset = spot_local(record.direction, offset);
    }
    let hit = cube_face(offset);
    // A spot writes one face, like `CensusKind::Spot`. `kind` mirrors
    // `GpuLight::kind`, and the order there is DIRECTIONAL 0, POINT 1,
    // SPOT 2 — not the order a reader guesses.
    let face = select(u32(hit.w), 0u, spot);
    let cell = vec2<u32>(clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side));

    let index = view_base()
        + light * pages.strides.z
        + face * pages.strides.y
        + level_base(level)
        + cell.y * side
        + cell.x;
    return vec2<u32>(index, level);
}

// One page of a local light's mip chain, marked.
fn mark_local(light: u32, world: vec3<f32>, wanted: f32) -> vec2<u32> {
    let page = local_page_for(light, world, wanted);
    // What THIS receiver asks of the page, on the sun's octave scale —
    // the number that decides which survivor list (which LOD) draws
    // into it. Min across receivers: the finest ask wins. See the
    // fourth word's doc beside `PAGE_CELL`.
    if page.x < pages.pool.x {
        let octave = page_octave(
            wanted,
            pages.eye_and_base.w,
            pages.chain.y,
            pages.chain.w,
        );
        atomicMin(&table_cells[page.x * PAGE_CELL + 3u], octave + 1u);
    }
    // 🔴 CLAIMED now, and the flag was a guard rather than an oversight.
    // A page claimed is a page in the table, and a page in the table
    // takes a pool slot from whoever else wanted one — with the census
    // asking for a thousand-odd local pages against a slice of a few
    // hundred, claiming them while nothing drew them evicted the sun
    // and left the frame with no shadows at all.
    //
    // What makes it safe is that the rest of the chain now exists: the
    // compaction buckets them by octave, the expansion tests them
    // against the lamp's own frustum, and the depth pass builds that
    // frustum from the light the page names. Turned on last, on
    // purpose.
    mark_bit(page.x, true);
    return page;
}

// One page of the sun's clipmap.
//
// Every level is a full grid rather than half of the last — that is what
// a clipmap is and what a mip chain is not — so the offset is a multiply
// where `mark_local`'s is a running sum.
// Which page of the sun's clipmap a point belongs to, WITHOUT marking
// it.
//
// 🔴 Split out so the paint pass can ask the same question a frame
// later. The paint runs after the shading now — see `paint_view` — and
// a second copy of this arithmetic there is a debug view that draws a
// page the marking never chose, which is worse than no view at all.
fn sun_page_for(slot: u32, world: vec3<f32>, wanted: f32) -> vec2<u32> {
    let basis = sun_basis(pages.sun.xyz);
    let eye = pages.eye_and_base.xyz;
    let base = pages.eye_and_base.w;
    let side = pages.strides.x;
    let texels = f32(pages.chain.y);

    // Containment is judged from the camera and the cell from the
    // SNAPPED grid — see `sun_level` for the slack that costs.
    let plane = sun_plane(world, basis) - sun_plane(eye, basis);
    let reach = max(abs(plane.x), abs(plane.y)) * 2.0;
    let contain = f32(sun_level(reach, base, side));
    // Containment is a ceiling on how far the sample is, density a floor
    // on how fine the level may be. Mirrors `mark_sun_cell`.
    let density = select(0.0, floor(log2(max(wanted * texels / base, 1.0))), wanted * texels > base);
    let level = min(u32(max(contain, density)), pages.chain.w - 1u);

    let extent = base * exp2(f32(level));
    let centre = sun_centre(eye, basis, base, side, level);
    let uv = clamp(
        (sun_plane(world, basis) - centre) / extent + vec2<f32>(0.5),
        vec2<f32>(0.0),
        vec2<f32>(0.99999),
    );
    let cell = vec2<u32>(uv * f32(side));

    let index = view_base()
        + slot * pages.strides.z
        + level * side * side
        + cell.y * side
        + cell.x;
    return vec2<u32>(index, level);
}

// One page of the sun's clipmap, marked.
fn mark_sun(slot: u32, world: vec3<f32>, wanted: f32) -> vec2<u32> {
    let page = sun_page_for(slot, world, wanted);
    mark_bit(page.x, true);
    return page;
}

// The colour a page is painted.
//
// 🔴 Two signals in one pixel, because either alone answers half the
// question. **Hue is the level** — where the frame is spending detail,
// and a sudden band of it is a level boundary. **Brightness is the page
// identity**, hashed, so neighbouring pages differ and the tiling is
// visible; a page that covers a quarter of the screen is a page too
// coarse for it, and a mosaic too fine to resolve is detail nobody sees.
fn page_color(index: u32, level: u32) -> vec3<f32> {
    var base = vec3<f32>(0.6);
    switch level % 6u {
        case 0u: { base = vec3<f32>(1.0, 0.25, 0.25); }
        case 1u: { base = vec3<f32>(1.0, 0.65, 0.2); }
        case 2u: { base = vec3<f32>(0.9, 0.95, 0.25); }
        case 3u: { base = vec3<f32>(0.3, 0.9, 0.4); }
        case 4u: { base = vec3<f32>(0.3, 0.6, 1.0); }
        default: { base = vec3<f32>(0.75, 0.4, 1.0); }
    }
    // A cheap integer hash, so adjacent page indices land on visibly
    // different values rather than on a gradient.
    var h = index;
    h = (h ^ 61u) ^ (h >> 16u);
    h = h + (h << 3u);
    h = h ^ (h >> 4u);
    h = h * 0x27d4eb2du;
    h = h ^ (h >> 15u);
    return base * (0.45 + 0.55 * f32(h & 0xffu) / 255.0);
}

@compute @workgroup_size(MARK_GROUP, MARK_GROUP, 1)
fn mark_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let rate = max(pages.sampling.x, 1u);
    let pixel = id.xy * rate;
    let size = vec2<u32>(view.viewport.xy);
    if pixel.x >= size.x || pixel.y >= size.y {
        return;
    }

    // 🔴 Reversed-Z infinite (ADR 0002): the buffer clears to 0 and that
    // is the FAR value, so a zero is sky rather than a surface at the
    // near plane. Marking it would put a page under every pixel the
    // scene does not cover, which is the whole failure this pass exists
    // to avoid.
    let depth = textureLoad(depth_tex, vec2<i32>(pixel), 0);
    if depth <= 0.0 {
        return;
    }
    atomicAdd(&counters[1], 1u);

    let uv = (vec2<f32>(pixel) + vec2<f32>(0.5)) / view.viewport.xy;
    let ndc = vec3<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, depth);
    let hom = pages.world_from_clip * vec4<f32>(ndc, 1.0);
    if abs(hom.w) < 1e-9 {
        return;
    }
    let world = hom.xyz / hom.w;

    // World metres one screen pixel covers here, from the frustum rather
    // than from the froxel: `2 * d * tan(fov/2) / height`, with the
    // focal length read off the projection. Mirrors
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

    if pages.sun.w > 0.5 {
        _ = mark_sun(pages.sampling.y, world, wanted);
    }

    if view.dimensions.w == 0u {
        return;
    }
    let cell = cluster_of_ndc(view, ndc, view_pos.z);
    let record = cells[cluster_index(cell, view.dimensions)];
    let start = record.offset;
    // Points and spots are the first two ranges, stored in that order,
    // and both need pages. Probes, volumes and decals do not.
    let count = record.point_count + record.spot_count;
    for (var i = 0u; i < count; i = i + 1u) {
        let slot = start + i;
        if slot >= arrayLength(&indices) {
            break;
        }
        let light = indices[slot];
        if light >= pages.strides.w {
            continue;
        }
        atomicAdd(&counters[2], 1u);
        _ = mark_local(light, world, wanted);
    }
}

// Paints the page each pixel chose, over the frame's final colour.
//
// # 🔴 Its own dispatch, and it runs AFTER the shading
//
// The marking moved to the top of the frame so the raster can fill the
// atlas before anything samples it. The paint cannot go with it: it
// writes the view's FINAL colour buffer, and at the top of the frame
// that buffer still holds the last frame's image, which the fused pass
// is about to overwrite. So the debug view would be painted and then
// erased, every frame, and read as "the view does not work".
//
// It asks `sun_page_for` the same question the marking asked, rather
// than carrying the answer forward in a buffer the size of the screen.
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

    // 🔴 One page per pixel, and the sun wins when there is one: a pixel
    // is lit by many lights, and painting the last one walked would make
    // the view depend on the light list's order.
    if pages.sun.w > 0.5 {
        paint_page(pixel, sun_page_for(pages.sampling.y, world, wanted));
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
        paint_page(pixel, local_page_for(light, world, wanted));
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

// Ages this view's table entries, and evicts the ones nothing has
// asked for in `max_age` frames.
//
// One thread per entry of THIS VIEW'S span — the table is flat and a
// view's entries are a contiguous run, so ownership is the dispatch
// range rather than a decode. A page nothing requested this frame is
// not gone, it is one frame older, and it stays resident — with its
// depth still in the atlas — until it has been unwanted for long
// enough to be worth the slot. Epic calls the threshold
// `MaxPageAgeSinceLastRequest`.
//
// Eviction stores `PAGE_ABSENT`, and that is the whole of it: nothing
// probes past an entry any more, so there is nothing a freed entry can
// break and no tombstone or sweep to keep the walk honest.
@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn age_view(@builtin(global_invocation_id) id: vec3<u32>) {
    let within = id.x;
    if within >= view_span() { return; }
    let entry = view_base() + within;
    // The receivers' ask is a PER-FRAME measurement: cleared here, on
    // the pass that already walks this view's entries and runs before
    // the marking, so each frame's minimum is this frame's. 0 means
    // "nobody asked", which the compaction reads as "fall back to the
    // range" — the right answer for a resident page no pixel is
    // looking at.
    atomicStore(&table_cells[entry * PAGE_CELL + 3u], 0u);
    let stored = atomicLoad(&table_cells[entry * PAGE_CELL]);
    if stored == PAGE_ABSENT { return; }

    // A rebuild empties the view outright — the pool's shape changed
    // under it, so a slot in an old entry names a page in a new atlas.
    if pages.life.z == 0u {
        let age = page_age(entry);
        // Unsigned, so a frame index that ran backwards (a rebuild, a
        // wrap) reads as enormous and evicts. That is the safe way for
        // this comparison to be wrong.
        if pages.life.x - age <= pages.life.y {
            atomicAdd(&counters[11], 1u);
            return;
        }
    }
    atomicStore(&table_cells[entry * PAGE_CELL], PAGE_ABSENT);
    page_release(stored - 1u);
    atomicAdd(&counters[12], 1u);
}
