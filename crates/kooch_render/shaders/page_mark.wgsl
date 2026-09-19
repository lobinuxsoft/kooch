// page_mark.wgsl — which shadow pages a frame actually needs (#866).

// Mirrors `PageMarkView` in `pages/mark.rs`, field for field.
struct PageView {
    world_from_clip: mat4x4<f32>,
    // xyz the camera, w the clipmap's level-0 extent in metres.
    eye_and_base: vec4<f32>,
    // xyz the sun's direction, w 1 if there is one.
    sun: vec4<f32>,
    // x page texels, y virtual texels, z levels in a local chain, w levels in the clipmap.
    chain: vec4<u32>,
    // x pages per side at level 0, y pages in one face's whole chain, z the per-light stride in
    // pages, w the light count.
    strides: vec4<u32>,
    // x the sampling rate in pixels, y the sun's slot WITHIN a view,
    // z 1 when the debug view is painting, w which view this is.
    sampling: vec4<u32>,
    // x entries in the page table, y physical pages the pool holds,
    // z pages across the atlas, w the pool slots ONE VIEW owns.
    pool: vec4<u32>,
    // xy how many output pixels one depth pixel covers, zw the output size.
    paint: vec4<f32>,
    // x THIS FRAME's index, y how many frames a page may go unrequested before it is evicted, z 1
    // when the pool is being rebuilt from nothing, w the sun's half-span in metres, bitcast (#949).
    life: vec4<u32>,
    // x the RECIPROCAL of `shadow_density`, as a fraction of 1, y the projected radius under which
    // a local light is DISTANT, z 1 when the per-light loop runs on froxels, w the distance gate in
    // multiples of a light's own range.
    density: vec4<f32>,
    // x how far, in PAGES, a receiver dilates its request; 0 = off.
    halo: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: ClusterView;
// The read side of `ClusterCell`, without the atomics.
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
@group(0) @binding(8) var color_out: texture_storage_2d<rgba8unorm, write>;

// The page table, FLAT: the entry index IS the virtual page id, so the lookup the shading pass runs
// per pixel per light is one load. `page_table.wgsl` holds the id arithmetic and the atlas layout —
// everything the READER has to agree with, kept in one file so the two cannot drift.
@group(0) @binding(10) var<storage, read_write> table_cells: array<atomic<u32>>;
// The allocator's own state, laid out per view: `[high, free_count, free_slots...]` repeated every
// `slice + 2` words.
@group(0) @binding(11) var<storage, read_write> alloc: array<atomic<u32>>;
// The seating plan, one run of `RANK_WORDS` per view: a demand histogram by rank, then the cutoff
// the plan chose, the quota left within the cutoff rank, and the spare the unmarked cache may keep.
// Cleared per view per frame — demand is a frame's question. #942.
@group(0) @binding(9) var<storage, read_write> rank_state: array<atomic<u32>>;

// ── Rank: who is seated when the frame wants more than the slice ──
const RANKS: u32 = 32u;
// 🔴 40 words of plan, then the OCCUPANCY BITMAP: one bit per froxel of this view, set by any pixel
// that lands in it.
const RANK_WORDS: u32 = 8360u;
/// First word of the occupancy bitmap, `OCCUPANCY_WORDS` long.
const RANK_OCCUPANCY: u32 = 40u;
const OCCUPANCY_WORDS: u32 = 128u;
/// Froxels the bitmap can hold. `ClusterSettings::default().total` is 4096; a grid larger than this
/// simply stops recording occupancy, which costs the census its meaning and never correctness.
const OCCUPANCY_MAX: u32 = 4096u;
/// Two words per froxel — the nearest and furthest view-space depth any sample of it reached, as
/// ordered bits.
const RANK_DEPTH: u32 = 168u;
const DEPTH_WORDS: u32 = 8192u;
const RANK_CUTOFF: u32 = 32u;
const RANK_QUOTA: u32 = 33u;
const RANK_SPARE: u32 = 34u;
// The resolution bias (#943), PERSISTENT across frames — the one word of a view's run the per-frame
// clear leaves alone.
const RANK_BIAS: u32 = 35u;
// Frames spent without pressure, also persistent.
const RANK_PATIENCE: u32 = 36u;
const PATIENCE_FRAMES: u32 = 16u;
// Locals give up four levels before the sun gives up one, and the sun stops at two: past that the
// pool is simply too small for the scene, and the panel says so through the denials that remain.
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
    // 🔴 The frame this page was ALLOCATED, and the only place it is written.
    atomicStore(&table_cells[entry * PAGE_CELL + 5u], frame);
}

fn page_refresh(entry: u32, frame: u32) {
    atomicStore(&table_cells[entry * PAGE_CELL + 1u], frame);
}

const NO_PAGE: u32 = 0xffffffffu;

const MARK_GROUP: u32 = 8u;

// The pages one view addresses, and where its own start.
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
fn alloc_base() -> u32 {
    return pages.sampling.w * (pages.pool.w + 2u);
}

// A physical slot out of this view's slice, recycled first.
fn page_alloc() -> u32 {
    let base = alloc_base();
    let slice = pages.pool.w;

    // Pop. `atomicSub` returning the OLD value is what makes the test and the take one operation: a
    // thread that sees a count of zero or less pushed it below zero and puts it back.
    let taken = atomicSub(&alloc[base + 1u], 1u);
    if taken != 0u && taken <= slice {
        atomicAdd(&counters[20], 1u);
        return atomicLoad(&alloc[base + 2u + taken - 1u]);
    }
    // 🔴 A pop that found nothing. Counted apart from the miss below, because the two mean opposite
    // things: this one says the list was empty (or that a concurrent popper drove the count
    // negative and this thread saw the underflow), while the miss says the bump is spent too.
    atomicAdd(&counters[23], 1u);
    atomicAdd(&alloc[base + 1u], 1u);

    let local = atomicAdd(&alloc[base], 1u);
    if local >= slice {
        atomicSub(&alloc[base], 1u);
        atomicAdd(&counters[5], 1u);
        return PAGE_MISS;
    }
    atomicAdd(&counters[21], 1u);
    return pages.sampling.w * slice + local;
}

// Gives a slot back to this view's free list.
fn page_release(slot: u32) {
    let base = alloc_base();
    let slice = pages.pool.w;
    let at = atomicAdd(&alloc[base + 1u], 1u);
    if at >= slice {
        // The list cannot hold more than the slice does, so this cannot happen without the slice
        // having been double-freed. Undo and leak the slot rather than write past the run.
        atomicSub(&alloc[base + 1u], 1u);
        atomicAdd(&counters[10], 1u);
        return;
    }
    atomicStore(&alloc[base + 2u + at], slot);
    atomicAdd(&counters[22], 1u);
}

// Finds `page` in the table, or puts it there, and stamps it with this frame either way.
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
    // Absent: demand only. Allocating here would be first-come-forever — the request that happened
    // to run first keeps its slot for the rest of the session while everyone else starves (#942,
    // measured at 6 652 requests starving against a 1 024 slice).
    return PAGE_MISS;
}

// One bit, set once. The return says whether this thread is the one that set it, which is what
// makes the counter a count of DISTINCT pages rather than of marking attempts — and what makes it
// the right place to allocate from.
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

// The coarsest level whose texels are still at least as dense as the screen's pixels. A cube face
// spans 90 degrees, so at `distance` it covers `2 * distance` world units across its texels.
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

// One page of a local light's mip chain. Which page of a local light's chain a point belongs to,
// WITHOUT marking it. Split for the same reason as `sun_page_for`.
fn local_page_for(light: u32, world: vec3<f32>, wanted: f32) -> vec2<u32> {
    let record = lights[light];
    var offset = world - record.position;
    let distance = max(length(offset), 0.05);
    // 🔴 A DISTANT light gets the top of its chain and nothing else,
    // which is one page for the whole face. See `light_distant`.
    let level = select(page_level(distance, wanted), pages.chain.z - 1u, light_distant(light));
    let side = level_side(level);

    // A spot's one face is aligned with ITS axis, not the world's — see `spot_local` for the
    // three-way disagreement this rotation ended.
    let spot = record.kind == PAGE_KIND_SPOT;
    if spot {
        offset = spot_local(record.direction, offset);
    }
    let hit = cube_face(offset);
    // A spot writes one face, like `CensusKind::Spot`. `kind` mirrors `GpuLight::kind`, and the
    // order there is DIRECTIONAL 0, POINT 1, SPOT 2 — not the order a reader guesses.
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
    // 🔴 CLAIMED now, and the flag was a guard rather than an oversight.
    if mark_bit(page.x, true) {
        atomicAdd(&rank_state[rank_base() + rank_local(page.y)], 1u);
    }
    return page;
}

// One page of the sun's clipmap.
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

/// A dilated request: residency only, no receiver bound.
fn mark_sun_halo(slot: u32, world: vec3<f32>, wanted: f32, centre: u32) {
    let page = sun_page_for(slot, world, wanted);
    if page.x == centre {
        return;
    }
    if mark_bit(page.x, true) {
        atomicAdd(&rank_state[rank_base() + rank_sun(page.y)], 1u);
    }
}

// One page of the sun's clipmap, marked.
fn mark_sun(slot: u32, world: vec3<f32>, wanted: f32, dither: vec2<f32>) -> vec2<u32> {
    let page = sun_page_for(slot, world, wanted);
    // lamp's radius. `mark_local` records how FAR a receiver is; a lamp is a point, so distance
    // alone bounds it and any direction may occlude.
    if page.x < pages.pool.x {
        let basis = sun_basis(pages.sun.xyz);
        let eye = pages.eye_and_base.xyz;
        // The SAME origin the expansion measures a caster from: the level's snapped depth grid, not
        // the raw camera. Measuring the two ends of one comparison from different origins is what
        // #948 cost a day over.
        let along = dot(world - eye, basis[2])
            + sun_drift(eye, basis, pages.eye_and_base.w, pages.strides.x, page.y);
        let span = bitcast<f32>(pages.life.w);
    }
    if mark_bit(page.x, true) {
        atomicAdd(&rank_state[rank_base() + rank_sun(page.y)], 1u);
    }
    // The halo, in the sun's own plane.
    let halo = pages.halo.x;
    if halo > 0.0 {
        let basis = sun_basis(pages.sun.xyz);
        let width = pages.eye_and_base.w * exp2(f32(page.y)) / f32(max(pages.strides.x, 1u));
        // 🔴 `dither` is why this covers a ring and not a line.
        let step = (basis[0] * dither.x + basis[1] * dither.y) * (halo * width);
        mark_sun_halo(slot, world + step, wanted, page.x);
        mark_sun_halo(slot, world - step, wanted, page.x);
    }
    return page;
}

// The colour a page is painted.
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
var<workgroup> tally: array<atomic<u32>, 5>;
const TALLY_SAMPLES: u32 = 0u;
const TALLY_PAIRS: u32 = 1u;
const TALLY_CULLED: u32 = 2u;
/// The worst overlap — a MAX, not a sum, so the flush is a max too.
const TALLY_PEAK: u32 = 3u;
/// Pairs served by the DISTANT tier — one page per face rather than a chain (#1009). Counted for
/// the same reason `TALLY_CULLED` is: a lamp demoted to one page and a lamp that never got a slot
/// both show up as a soft, low-resolution shadow, and only this number tells them apart.
const TALLY_DISTANT: u32 = 4u;

