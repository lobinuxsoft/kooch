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
// The seating plan, one run of `RANK_WORDS` per view: a demand
// histogram by rank, then the cutoff the plan chose, the quota left
// within the cutoff rank, and the spare the unmarked cache may keep.
// Cleared per view per frame — demand is a frame's question. #942.
//
// ⚠️ This spends the last storage-buffer slot under the
// eight-per-stage downlevel limit the layout's own comment reserves.
@group(0) @binding(9) var<storage, read_write> rank_state: array<atomic<u32>>;

// ── Rank: who is seated when the frame wants more than the slice ──
//
// Smaller rank = seated first. The sun's clipmap ranks ahead of every
// local light — the one consumer every frame has, and the consumer
// whose loss the last starvation made unmissable — and within any
// chain the COARSE levels rank ahead of the fine, so under pressure a
// consumer loses its finest detail before it loses coverage. The
// alternative — interleaving sun and local octaves — would trade
// near-camera sun detail for lamp pages, which is a regression against
// what the pool does today; revisit when #943's bias makes demand fit.
const RANKS: u32 = 32u;
// 🔴 40 words of plan, then the OCCUPANCY BITMAP: one bit per froxel of
// this view, set by any pixel that lands in it.
//
// Olsson §III computes shadow resolution and page masks from
// **cluster/light pairs**, not sample/light pairs, because cluster
// bounding boxes are "several orders of magnitude fewer than the
// samples". Measured here: 3 369 702 sample/light pairs against 218 772
// covered pixels — 15.4 lights per pixel — for a grid capped at 4096
// froxels.
//
// But the grid assigns lights to froxels GEOMETRICALLY and knows nothing
// about what is visible, while the per-pixel marking is occlusion-culled
// for free: an occluded pixel does not exist and marks nothing. A
// cluster pass without this bitmap would mark pages for empty air and
// for froxels behind walls, spending a pool the panel already reports
// 100% full. This is what keeps the depth buffer's answer.
//
// Rides in `rank_state` rather than a binding of its own: the pass sits
// at the eight-storage-buffer downlevel limit exactly.
const RANK_WORDS: u32 = 8360u;
/// First word of the occupancy bitmap, `OCCUPANCY_WORDS` long.
const RANK_OCCUPANCY: u32 = 40u;
const OCCUPANCY_WORDS: u32 = 128u;
/// Froxels the bitmap can hold. `ClusterSettings::default().total` is
/// 4096; a grid larger than this simply stops recording occupancy, which
/// costs the census its meaning and never correctness.
const OCCUPANCY_MAX: u32 = 4096u;
/// Two words per froxel — the nearest and furthest view-space depth any
/// sample of it reached, as ordered bits.
///
/// 🔴 Olsson Fig. 8 calls these the EXPLICIT bounds, against the
/// IMPLICIT ones a froxel's own box gives, and the distinction is the
/// whole difference between the cluster path working and not. A froxel
/// is mostly empty: the occupancy bit says a pixel landed in it, but the
/// surface inside is a thin sheet and its box is a slab. Marking the box
/// asks for pages across a depth range that holds nothing.
///
/// Measured: with implicit bounds the resolution bias went straight to
/// its ceiling — `locals +4 · sun +2`, both maxed, 21 pages denied —
/// against `locals +1 · sun +0` on the per-pixel path. The walk was
/// 534x cheaper and the pool paid all of it back.
const RANK_DEPTH: u32 = 168u;
const DEPTH_WORDS: u32 = 8192u;
const RANK_CUTOFF: u32 = 32u;
const RANK_QUOTA: u32 = 33u;
const RANK_SPARE: u32 = 34u;
// The resolution bias (#943), PERSISTENT across frames — the one word
// of a view's run the per-frame clear leaves alone. Low byte the local
// lights' bias in levels, next byte the sun's: each step doubles the
// world size a screen pixel may ask a shadow texel to match, which is
// one level coarser and a QUARTER of the pages. `bias_view` moves it
// one step per frame; the readers walk their chains from the fine end,
// so a coarser marking is found without their code knowing a bias
// exists.
const RANK_BIAS: u32 = 35u;
// Frames spent without pressure, also persistent. The strict unwind
// only fires when the arithmetic PROVES a finer marking fits; where it
// cannot prove it (coarse clipmap levels do not quadruple), the bias
// TRIES a step down once the patience runs out, and the ordinary raise
// reverts a failed trial the next frame. The still-resident coarser
// pages catch the readers while it fails, so a failed trial costs one
// frame of fallback, not a frame of missing shadow.
const RANK_PATIENCE: u32 = 36u;
const PATIENCE_FRAMES: u32 = 16u;
// Locals give up four levels before the sun gives up one, and the sun
// stops at two: past that the pool is simply too small for the scene,
// and the panel says so through the denials that remain.
const LOCAL_BIAS_MAX: u32 = 4u;
const SUN_BIAS_MAX: u32 = 2u;

fn rank_base() -> u32 {
    return pages.sampling.w * RANK_WORDS;
}

fn rank_sun(level: u32) -> u32 {
    return min(pages.chain.w - 1u - level, RANKS - 1u);
}

fn rank_local(level: u32) -> u32 {
    return min(pages.chain.w + (pages.chain.z - 1u - level), RANKS - 1u);
}

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
    // A fresh claim has no content: whatever the slot held belonged to
    // whoever held it last. Zero is "never drawn" to the cache.
    atomicStore(&table_cells[entry * PAGE_CELL + 3u], 0u);
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
    // Absent: demand only. Allocating here would be first-come-forever
    // — the request that happened to run first keeps its slot for the
    // rest of the session while everyone else starves (#942, measured
    // at 6 652 requests starving against a 1 024 slice). `adopt_view`
    // seats the frame's marked pages AFTER `plan_view` has ranked the
    // whole frame's demand.
    return PAGE_MISS;
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
    // #940: every sample is a RECEIVER, and the furthest one bounds
    // what can occlude anything on this page — a caster whose nearest
    // point lies beyond it shadows nobody the frame shades. Radial
    // distance rather than face depth, so the bound is face-agnostic
    // and errs toward keeping. Positive floats bitcast to ordered
    // u32s, which is what lets an atomicMax hold a distance.
    if page.x < pages.pool.x {
        let d = length(world - lights[light].position);
        atomicMax(
            &table_cells[page.x * PAGE_CELL + 4u],
            bitcast<u32>(max(d, 0.0)),
        );
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
    if mark_bit(page.x, true) {
        atomicAdd(&rank_state[rank_base() + rank_local(page.y)], 1u);
    }
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

    // Keyed by ABSOLUTE world page, wrapped. See `sun_cell` for why the
    // camera-relative key cost every page on every step.
    let cell = sun_cell(world, eye, basis, base, side, level);

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
    if mark_bit(page.x, true) {
        atomicAdd(&rank_state[rank_base() + rank_sun(page.y)], 1u);
    }
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

/// The census, accumulated per WORKGROUP and flushed once.
///
/// 🔴 These three used to be `atomicAdd(&counters[n], 1u)` straight into
/// global memory, from the innermost loop of a pass that runs one thread
/// per pixel: one increment per covered pixel and one per (pixel, light)
/// pair, every one of them landing on the same two addresses. At the
/// resolution the OneXFly captures at that is millions of increments
/// serialised on two words, in a pass measured at a flat 13.975 ms
/// (#952). They are diagnostics — the panel and #942's plan read them —
/// so the fix is to make them cheap, not to delete them.
///
/// A workgroup is `MARK_GROUP * MARK_GROUP` = 64 threads, so this trades
/// up to 64 global atomics for one.
var<workgroup> tally: array<atomic<u32>, 4>;
const TALLY_SAMPLES: u32 = 0u;
const TALLY_PAIRS: u32 = 1u;
const TALLY_CULLED: u32 = 2u;
/// The worst overlap — a MAX, not a sum, so the flush is a max too.
const TALLY_PEAK: u32 = 3u;

/// 🔴 The barriers live HERE and the work lives in `mark_pixel`, because
/// `workgroupBarrier` in non-uniform control flow is undefined and
/// `mark_pixel` returns early three ways — off the edge of the viewport,
/// on sky, on a degenerate reconstruction. Every thread of the group
/// reaches both barriers; only some of them do any marking.
@compute @workgroup_size(MARK_GROUP, MARK_GROUP, 1)
fn mark_main(
    @builtin(global_invocation_id) id: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    if lane < 4u {
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
    }
}

fn mark_pixel(id: vec3<u32>) {
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
    atomicAdd(&tally[TALLY_SAMPLES], 1u);

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
    // The pressure bias (#943): what last frame's plan learned, applied
    // to what this frame asks for.
    let bias = atomicLoad(&rank_state[rank_base() + RANK_BIAS]);
    let local_wanted = wanted * exp2(f32(bias & 0xffu));

    if pages.sun.w > 0.5 {
        _ = mark_sun(pages.sampling.y, world, wanted * exp2(f32(bias >> 8u)));
    }

    if view.dimensions.w == 0u {
        return;
    }
    let froxel = cluster_index(cluster_of_ndc(view, ndc, view_pos.z), view.dimensions);
    // This froxel holds visible surface. One `atomicOr` per pixel,
    // and it replaces nothing yet — see `RANK_OCCUPANCY` for what it
    // is for and why the depth buffer's answer has to be kept.
    if froxel < OCCUPANCY_MAX {
        atomicOr(
            &rank_state[rank_base() + RANK_OCCUPANCY + froxel / 32u],
            1u << (froxel % 32u),
        );
        // The explicit bounds: how deep this froxel's SURFACE actually
        // runs. Positive floats bitcast to u32 keep their order, which
        // is what lets an atomic hold a depth.
        // ⚠️ The NEAR end is stored complemented so that both words are
        // an `atomicMax` and zero is the identity for both. The per-frame
        // clear writes zeroes and nothing else can, so an `atomicMin`
        // against a cleared word would never move off zero — the nearest
        // depth in the scene would read as the camera itself.
        let depth_bits = bitcast<u32>(abs(view_pos.z));
        let slab = rank_base() + RANK_DEPTH + froxel * 2u;
        atomicMax(&rank_state[slab], ~depth_bits);
        atomicMax(&rank_state[slab + 1u], depth_bits);
    }
    let record = cells[froxel];
    // Recorded on BOTH paths: the overlap is a property of the scene, not
    // of how the marking walks it, and the alert has to mean the same
    // thing whichever is on. Into WORKGROUP memory — one thread per
    // pixel lands here, and `the_hot_path_counts_in_workgroup_memory`
    // exists to stop exactly the global atomic this was on its first
    // draft.
    atomicMax(&tally[TALLY_PEAK], record.point_count + record.spot_count);
    // 🔴 The per-light walk belongs to `mark_froxels` when the cluster
    // path is on (#952). Everything above this line still runs: the sun
    // is marked per pixel, and the occupancy bit that pass reads was set
    // above.
    if pages.density.z > 0.5 {
        return;
    }
    let start = record.offset;
    // Points and spots are the first two ranges, stored in that order,
    // and both need pages. Probes, volumes and decals do not.
    let count = record.point_count + record.spot_count;
    // 🔴 Counted in registers first. Even an LDS atomic is a shared
    // address, and this loop body runs once per light per pixel — the
    // hottest place in the pass. One thread's whole cluster costs it two
    // workgroup atomics at the end instead of two per light.
    var pairs = 0u;
    var culled = 0u;
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
        // The coverage gate (#944): a light whose WHOLE range projects
        // under the threshold casts no pages — it still shades, and the
        // reader finds nothing and returns lit. Epic runs the same rule
        // as a pass, `PruneLightGridCS`, before anything marks; here it
        // is a comparison the loop already has every operand for.
        if pages.density.y > 0.0 && coverage_pixels(light) < pages.density.y {
            culled = culled + 1u;
            continue;
        }
        _ = mark_local(light, world, local_wanted);
    }
    if pairs != 0u {
        atomicAdd(&tally[TALLY_PAIRS], pairs);
    }
    if culled != 0u {
        atomicAdd(&tally[TALLY_CULLED], culled);
    }
}

// The projected radius of a light's range sphere, in screen pixels —
// the whole reach of the light, not the lit part of it, so the gate
// errs toward casting. Camera-dependent and pixel-independent: the
// same number every sample computes.
fn coverage_pixels(light: u32) -> f32 {
    let record = lights[light];
    let distance = max(length(record.position - pages.eye_and_base.xyz), 0.05);
    let focal = view.clip_from_view[1][1];
    return record.range * abs(focal) * view.viewport.y / (2.0 * distance);
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
    // The same bias the marking applied, or the view paints pages the
    // marking never chose — the failure `sun_page_for` was split to end.
    let bias = atomicLoad(&rank_state[rank_base() + RANK_BIAS]);

    // 🔴 One page per pixel, and the sun wins when there is one: a pixel
    // is lit by many lights, and painting the last one walked would make
    // the view depend on the light list's order.
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
        if pages.density.y > 0.0 && coverage_pixels(light) < pages.density.y {
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
    // The receiver bound (#940) is a per-FRAME quantity: this pass is
    // the one thread per entry that already runs first, so the reset
    // rides it. Zero means "no receiver recorded", which every reader
    // treats as "never reject".
    atomicStore(&table_cells[entry * PAGE_CELL + 4u], 0u);
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

// Turns the demand histogram into a seating plan: how deep down the
// ranks this view's slice reaches. One thread — RANKS is 32 and the
// loop IS the prefix sum; a parallel scan here would cost more to
// coordinate than it computes.
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

// Clears the seats the plan did not fund. Runs AFTER the plan and
// BEFORE `adopt_view`, and that order is the stability: a resident of
// the cutoff rank claims the quota ahead of any newcomer, so at equal
// importance a page WITH content beats a page without and a constant
// demand keeps a constant resident set.
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
            // Same take-then-test as `page_alloc`: the old value says
            // whether the take was funded.
            let quota = atomicSub(&rank_state[base + RANK_QUOTA], 1u);
            if quota != 0u && quota <= pages.pool.w { return; }
            atomicAdd(&rank_state[base + RANK_QUOTA], 1u);
        }
    } else {
        // Unrequested this frame. It stays as CACHE only while the
        // slice has room after the frame's own demand is seated —
        // `age_view` already dropped what went stale, this is the
        // pressure valve on the rest. Epic keeps cached pages the same
        // way: only in the space the requests leave over.
        let spare = atomicSub(&rank_state[base + RANK_SPARE], 1u);
        if spare != 0u && spare <= pages.pool.w { return; }
        atomicAdd(&rank_state[base + RANK_SPARE], 1u);
    }
    atomicStore(&table_cells[entry * PAGE_CELL], PAGE_ABSENT);
    page_release(stored - 1u);
    atomicAdd(&counters[14], 1u);
}

// Seats this frame's marked pages: everything under the cutoff gets a
// slot, the cutoff rank competes for what the residents left of the
// quota, and everything past it is DENIED and counted — the panel's
// answer to who the slice turned away. The arithmetic closes by
// construction: the plan funded at most `budget` seats and the
// preemption freed everything unfunded, so a funded request cannot
// find the free list empty.
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

// Moves the resolution bias one step per frame toward the coarsest
// marking that fits the slice (#943). UE5 runs the same loop as its
// page-pool-overflow bias; Olsson caps every light's resolution by
// projected area (Eq. 1) for the same reason: when demand cannot fit,
// the answer is to serve EVERYONE coarser, not to turn 87 % of the
// requests away.
//
// Under pressure the locals pay first and the sun only when they have
// nothing left to give; with headroom the sun recovers first. A step
// down is taken only when the slack covers ~3x that party's current
// demand — one level finer is four times the pages, and that margin is
// the hysteresis that keeps a constant demand at a constant bias.
@compute @workgroup_size(1, 1, 1)
fn bias_view() {
    let base = rank_base();
    let word = atomicLoad(&rank_state[base + RANK_BIAS]);
    var local_bias = word & 0xffu;
    var sun_bias = word >> 8u;
    let cutoff = atomicLoad(&rank_state[base + RANK_CUTOFF]);
    var patience = atomicLoad(&rank_state[base + RANK_PATIENCE]);
    let budget = pages.pool.w;

    // 🔴 Read on BOTH branches now. The raise used to step by one and
    // wait for the next frame to see whether that was enough, so a scene
    // needing four steps took four frames to stop denying and up to
    // ninety-six to give them back. The demand is right here; the number
    // of steps it implies can be computed instead of discovered.
    //
    // WickedEngine does the same thing without any lag at all
    // (`wiRenderer.cpp`): it sizes every light by `min(1, range/dist)`,
    // tries to pack, and on failure halves everything and repacks —
    // inside the frame, from no state. It can, because its sizes come
    // from a formula. Ours are MEASURED by a per-pixel pass, so a second
    // evaluation would mean a second marking; one frame of lag is the
    // floor here, and one frame is what this gets it to.
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

    if cutoff < RANKS {
        // ⚠️ Four pages become one per step is the OPTIMISTIC estimate,
        // and it is chosen on purpose. Raising too little costs one more
        // frame of denials; raising too much costs blur the player sees.
        // The error belongs on the low side.
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
        // ⚠️ Growth is FOUR times a level here, the pessimistic estimate,
        // for the same reason inverted: an unwind that over-reaches is
        // the blur coming straight back next frame.
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

// How many froxels of this view hold visible surface — the population of
// the occupancy bitmap `mark_pixel` fills.
//
// 🔴 The number that sizes the move to cluster/light pairs. The marking
// runs per (pixel, light) and measures 3 369 702 pairs a frame; a pass
// over occupied froxels would run per (froxel, light) instead, and this
// is the multiplier. Counted rather than assumed: the grid is capped at
// 4096 and the fraction of it a scene actually occupies is the whole
// question.
@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn count_froxels(@builtin(local_invocation_index) lane: u32) {
    let base = rank_base() + RANK_OCCUPANCY;
    var found = 0u;
    // 64 lanes over `OCCUPANCY_WORDS`, strided so the loop is the same
    // length in every lane.
    for (var w = lane; w < OCCUPANCY_WORDS; w = w + MARK_GROUP * MARK_GROUP) {
        found = found + countOneBits(atomicLoad(&rank_state[base + w]));
    }
    if found != 0u {
        atomicAdd(&counters[9], found);
    }
}

// ---------------------------------------------------------------------
// Marking per (froxel, light) instead of per (pixel, light) — #952.
//
// Olsson §III: "the cost can be reduced substantially by using
// cluster/light pairs in place of sample/light pairs", because cluster
// bounds are "several orders of magnitude fewer than the samples".
// Measured in `many_lights`: 2 937 330 sample/light pairs against 199
// occupied froxels at 17.9 lights each — **824x fewer**.
//
// # 🔴 The coarsest corner decides the level, and that is what makes it safe
//
// A froxel's eight corners want different levels: the near one is closer
// to the light and asks for finer texels. Marking each corner's own level
// would need every one of them resident. Marking the COARSEST is always
// resolvable instead, because the readers walk their chain from the fine
// end upward — `inti_pbr`'s loop is `for (; level < chain.x; level++)` —
// so a pixel that wanted level L finds a page at L+1 and shades with it.
// The cost is resolution, not correctness, and it is the same trade
// #943's bias already makes under pressure.
//
// # The rect, and why it is filled rather than sampled
//
// Eight corner lookups give eight cells. The volume between them needs
// pages too, so each face marks the whole rect its corners span. That
// over-marks — the froxel is a frustum and the rect is its bounding box
// on the cube face — which is the conservative direction: a page nobody
// samples costs a pool slot, a page nobody marked costs a shadow.

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
    // Occupied only. The bitmap is what keeps the depth buffer's free
    // occlusion culling: a froxel of empty air, or one behind a wall,
    // has no bit and marks nothing.
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

    // The grid coordinate back out of the flat index; mirrors
    // `cluster_index`.
    let dz = max(view.dimensions.z, 1u);
    let dx = max(view.dimensions.x, 1u);
    let cell = vec3<u32>((froxel / dz) % dx, (froxel / dz) / dx, froxel % dz);
    var bounds = cluster_cell_bounds(view, cell);
    // 🔴 The froxel's box narrowed to the slab its SURFACE occupies —
    // Olsson's explicit bounds against its implicit ones. Without this
    // the pass marks pages for the empty depth either side of a thin
    // sheet, and the pool pays for every one of them.
    let slab = rank_base() + RANK_DEPTH + froxel * 2u;
    let far_bits = atomicLoad(&rank_state[slab + 1u]);
    if far_bits != 0u {
        let near_bits = ~atomicLoad(&rank_state[slab]);
        // View space looks down -Z, so the nearer surface is the LARGER
        // z. Clamped INTO the froxel's own box, never outside it: the
        // slab is what the samples reached and the box is what the
        // addressing says this cell covers.
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
    // What a screen pixel covers at this froxel's FAR face — the same
    // quantity `mark_pixel` computes per sample, evaluated once for the
    // whole cell and at its coarsest end.
    let focal = view.clip_from_view[1][1];
    var wanted = 0.0;
    if abs(focal) > 1e-9 {
        wanted = 2.0 * abs(bounds.min.z) / (focal * max(view.viewport.y, 1.0));
    }
    let bias = atomicLoad(&rank_state[rank_base() + RANK_BIAS]);
    wanted = wanted * pages.density.x * exp2(f32(bias & 0xffu));

    var pairs = 0u;
    var culled = 0u;
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
        // 🔴 COUNTED, not merely skipped. A lamp under the projected-size
        // gate (#944) casting nothing is indistinguishable from a lamp
        // that was never reached, and the panel reads this number to tell
        // "the gate is working" from "the light is missing". The
        // per-pixel path counts it; a second path that quietly did not
        // would make the two disagree for a reason nothing states.
        if pages.density.y > 0.0 && coverage_pixels(light) < pages.density.y {
            culled = culled + 1u;
            continue;
        }
        mark_froxel_light(light, corners, wanted);
    }
    // One thread per FROXEL, thousands rather than millions, so these go
    // straight to the counters — the workgroup reduction `mark_pixel`
    // needs buys nothing at this width. See `mark_flush`.
    if pairs != 0u {
        atomicAdd(&counters[2], pairs);
    }
    if culled != 0u {
        atomicAdd(&counters[6], culled);
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
///
/// `local_page_for` picks the level from the distance; this takes it as
/// an argument, because a froxel's eight corners have to be compared on
/// one grid before a rect between them means anything.
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

/// Marks the cells `lo..=hi` of one face, coarsening until the rect fits
/// [`FROXEL_RECT_MAX`] on both axes.
///
/// 🔴 Coarsening rather than clamping the rect. Clamping would drop the
/// cells past the cap and take their shadows with them; a level coarser
/// halves the cells on each axis and still covers the whole froxel,
/// which is the trade the readers are built to absorb.
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
            // #940's receiver bound, from the froxel's furthest corner
            // rather than a sample: larger, so it rejects less, which is
            // the safe direction for a bound that culls casters.
            atomicMax(&table_cells[index * PAGE_CELL + 4u], bitcast<u32>(max(reach, 0.0)));
            // 🔴 `mark_bit`, not `page_touch`. `page_touch` only
            // refreshes an entry that already exists; the bit is what
            // makes a page EXIST, and what `plan_view` later ranks. The
            // first version of this called `page_touch` and marked
            // nothing at all — 169 pairs walked, zero pages resident.
            if mark_bit(index, true) {
                atomicAdd(&rank_state[rank_base() + rank_local(level)], 1u);
                page_touch(index);
            }
        }
    }
}
