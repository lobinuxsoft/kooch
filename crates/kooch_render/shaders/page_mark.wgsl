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

// The page table, open-addressed. `page_table.wgsl` holds the hash, the
// probe sequence and the atlas layout — everything the READER of this
// table has to agree with, kept in one file so the two cannot drift.
//
// Keys are `virtual_page + 1` so that a cleared buffer is an empty
// table: the reset is `clear_buffer`, not a pass.
@group(0) @binding(9) var<storage, read_write> table_keys: array<atomic<u32>>;
// TWO words per entry: the physical slot, then the frame it was last
// REQUESTED in. `page_slot` and `page_age` are the accessors.
//
// 🔴 Interleaved rather than two buffers, and the reason is a hard
// limit: `max_storage_buffers_per_shader_stage` is EIGHT on the
// downlevel defaults, and this pass was already at eight. A ninth
// binding fails `create_bind_group_layout` outright — the age has to
// live beside the slot or the pass does not exist.
//
// It also happens to be the better layout: a hit reads the slot and
// writes the age, and now they share a cache line.
@group(0) @binding(10) var<storage, read_write> table_cells: array<atomic<u32>>;
// The allocator's own state, laid out per view: `[high, free_count,
// free_slots...]` repeated every `slice + 2` words.
//
// 🔴 NOT cleared between frames, which is the difference between this
// and what came before. The bump high-water mark and the free list are
// what a page's residency survives on; a `clear_buffer` over them every
// frame is exactly the non-persistent pool this replaces.
@group(0) @binding(11) var<storage, read_write> alloc: array<atomic<u32>>;

// The two words of a table entry. `PAGE_CELL` is in `page_table.wgsl`
// because the READER indexes the same buffer.
fn page_slot(entry: u32) -> u32 {
    return atomicLoad(&table_cells[entry * PAGE_CELL]);
}

fn page_age(entry: u32) -> u32 {
    return atomicLoad(&table_cells[entry * PAGE_CELL + 1u]);
}

fn page_stamp(entry: u32, slot: u32, frame: u32) {
    atomicStore(&table_cells[entry * PAGE_CELL], slot);
    atomicStore(&table_cells[entry * PAGE_CELL + 1u], frame);
}

fn page_refresh(entry: u32, frame: u32) {
    atomicStore(&table_cells[entry * PAGE_CELL + 1u], frame);
}

const NO_PAGE: u32 = 0xffffffffu;

const MARK_GROUP: u32 = 8u;

// Mirrors `gpu_light.rs`. Spelled out because the census's twin in Rust
// reads the same constants from that file.
const LIGHT_KIND_SPOT: u32 = 2u;

// The pages one view addresses, and where its own start.
//
// A view's slots run `[0, sun_slot]` — every light plus the sun — so the
// span is one more than the sun's own index times the per-light stride.
// Derived rather than uploaded: a second copy of it in the uniform is a
// second thing to keep in step with `strides.z`.
fn view_span() -> u32 {
    return (pages.sampling.y + 1u) * pages.strides.z;
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
// 🔴 This REPLACED a function that only ever inserted, because the pool
// used to be emptied every frame and every page was therefore new. Now
// the common case is the first branch: the page is already resident,
// its age is refreshed, and NOTHING is allocated and nothing has to be
// rasterised again. `counters[7]` counts those and `counters[8]` counts
// the ones that really are new — the two together are what says whether
// persistence is doing anything.
//
// ⚠️ The lookup walks PAST tombstones and stops only at EMPTY. See
// `PAGE_DEAD`: stopping at a freed entry would declare a resident page
// missing and allocate a second slot for it.
fn page_touch(page: u32) -> u32 {
    let entries = pages.pool.x;
    if entries == 0u {
        return PAGE_MISS;
    }
    let frame = pages.life.x;
    var probe = page_probe(page, entries);
    var dead = PAGE_MISS;
    for (var i = 0u; i < PAGE_PROBES; i = i + 1u) {
        let key = atomicLoad(&table_keys[probe]);
        if key == page + 1u {
            page_refresh(probe, frame);
            atomicAdd(&counters[7], 1u);
            return page_slot(probe);
        }
        if key == PAGE_DEAD && dead == PAGE_MISS {
            // Remember the first hole; the run still has to be walked to
            // the end in case the key lives further along it.
            dead = probe;
            atomicAdd(&counters[9], 1u);
        }
        if key == PAGE_EMPTY {
            break;
        }
        probe = page_step(probe, entries);
    }

    // Not resident. Take a slot first: an entry claimed with no slot to
    // put in it is an entry a reader finds and dereferences.
    let slot = page_alloc();
    if slot == PAGE_MISS {
        return PAGE_MISS;
    }

    // Reuse the hole when there was one, otherwise resume the walk. A
    // tombstone is claimed with the same compare-exchange as an empty
    // entry, so two threads cannot both take it.
    if dead != PAGE_MISS {
        let outcome = atomicCompareExchangeWeak(&table_keys[dead], PAGE_DEAD, page + 1u);
        if outcome.exchanged {
            page_stamp(dead, slot, frame);
            atomicAdd(&counters[8], 1u);
            return slot;
        }
        probe = dead;
    }

    for (var i = 0u; i < PAGE_PROBES; i = i + 1u) {
        let outcome = atomicCompareExchangeWeak(&table_keys[probe], PAGE_EMPTY, page + 1u);
        if outcome.exchanged {
            page_stamp(probe, slot, frame);
            atomicAdd(&counters[8], 1u);
            return slot;
        }
        // A failure with EMPTY still there is the "weak" in the name —
        // spurious, retry the same entry. Anything else is another key
        // holding it, and the sequence moves on.
        if outcome.old_value != PAGE_EMPTY {
            // Including our own key, if another thread inserted it while
            // this one was allocating. Hand the spare slot back.
            if outcome.old_value == page + 1u {
                page_release(slot);
                page_refresh(probe, frame);
                atomicAdd(&counters[7], 1u);
                return page_slot(probe);
            }
            probe = page_step(probe, entries);
        }
    }
    // The pool had room and the table did not, which is a statement
    // about the hash rather than about the scene.
    page_release(slot);
    atomicAdd(&counters[6], 1u);
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

// Where `level` starts inside one face's chain. Mirrors
// `PageConfig::level_base`: a mip chain's levels are not the same size,
// so the offset is a running sum and not a multiply.
fn level_base(level: u32) -> u32 {
    var base = 0u;
    var side = pages.strides.x;
    for (var l = 0u; l < level; l = l + 1u) {
        base = base + side * side;
        side = max(side / 2u, 1u);
    }
    return base;
}

fn level_side(level: u32) -> u32 {
    return max(pages.strides.x >> level, 1u);
}

// The coarsest level whose texels are still at least as dense as the
// screen's pixels. A cube face spans 90 degrees, so at `distance` it
// covers `2 * distance` world units across its texels.
fn page_level(distance: f32, wanted: f32) -> u32 {
    if wanted <= 0.0 {
        return 0u;
    }
    let texels = 2.0 * distance / wanted;
    if texels <= 0.0 {
        return pages.chain.z - 1u;
    }
    let level = floor(log2(f32(pages.chain.y) / texels));
    return min(u32(max(level, 0.0)), pages.chain.z - 1u);
}

// Which of the six cube faces a direction lands on, and its position
// across that face. Mirrors what `face_view_proj` produces without
// building the matrix: the major axis picks the face, and the other two
// divided by it are the face's normalised coordinates.
fn cube_face(dir: vec3<f32>) -> vec4<f32> {
    let a = abs(dir);
    var face = 0u;
    var uv = vec2<f32>(0.0);
    var major = 0.0;
    if a.x >= a.y && a.x >= a.z {
        major = a.x;
        face = select(1u, 0u, dir.x > 0.0);
        uv = select(vec2<f32>(dir.z, -dir.y), vec2<f32>(-dir.z, -dir.y), dir.x > 0.0);
    } else if a.y >= a.z {
        major = a.y;
        face = select(3u, 2u, dir.y > 0.0);
        uv = select(vec2<f32>(dir.x, -dir.z), vec2<f32>(dir.x, dir.z), dir.y > 0.0);
    } else {
        major = a.z;
        face = select(5u, 4u, dir.z < 0.0);
        uv = select(vec2<f32>(dir.x, -dir.y), vec2<f32>(-dir.x, -dir.y), dir.z < 0.0);
    }
    if major <= 0.0 {
        return vec4<f32>(0.5, 0.5, 0.0, f32(face));
    }
    return vec4<f32>(uv / major * 0.5 + vec2<f32>(0.5), 0.0, f32(face));
}

// One page of a local light's mip chain.
fn mark_local(light: u32, world: vec3<f32>, wanted: f32) -> vec2<u32> {
    let record = lights[light];
    let offset = world - record.position;
    let distance = max(length(offset), 0.05);
    let level = page_level(distance, wanted);
    let side = level_side(level);

    let hit = cube_face(offset);
    // A spot writes one face, like `CensusKind::Spot`. `kind` mirrors
    // `GpuLight::kind`, and the order there is DIRECTIONAL 0, POINT 1,
    // SPOT 2 — not the order a reader guesses.
    let face = select(u32(hit.w), 0u, record.kind == LIGHT_KIND_SPOT);
    let cell = vec2<u32>(clamp(hit.xy, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side));

    let index = view_base()
        + light * pages.strides.z
        + face * pages.strides.y
        + level_base(level)
        + cell.y * side
        + cell.x;
    // Marked, NOT claimed: nothing rasterises a local light's pages yet.
    mark_bit(index, false);
    return vec2<u32>(index, level);
}

// One page of the sun's clipmap.
//
// Every level is a full grid rather than half of the last — that is what
// a clipmap is and what a mip chain is not — so the offset is a multiply
// where `mark_local`'s is a running sum.
fn mark_sun(slot: u32, world: vec3<f32>, wanted: f32) -> vec2<u32> {
    let direction = normalize(pages.sun.xyz);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(direction.y) > 0.99 {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    // The light's basis, built rather than uploaded: the sun has no
    // position, so this is the only place it means anything.
    let f = direction;
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    let offset = world - pages.eye_and_base.xyz;
    let plane = vec2<f32>(dot(offset, s), dot(offset, u));

    let base = pages.eye_and_base.w;
    let texels = f32(pages.chain.y);
    let reach = max(abs(plane.x), abs(plane.y)) * 2.0;
    // Containment is a ceiling on how far the sample is, density a floor
    // on how fine the level may be. Mirrors `mark_sun_cell`.
    let contain = select(0.0, ceil(log2(max(reach / base, 1.0))), reach > base);
    let density = select(0.0, floor(log2(max(wanted * texels / base, 1.0))), wanted * texels > base);
    let level = min(u32(max(contain, density)), pages.chain.w - 1u);

    let extent = base * exp2(f32(level));
    let side = pages.strides.x;
    let uv = clamp(plane / extent + vec2<f32>(0.5), vec2<f32>(0.0), vec2<f32>(0.99999));
    let cell = vec2<u32>(uv * f32(side));

    let index = view_base()
        + slot * pages.strides.z
        + level * side * side
        + cell.y * side
        + cell.x;
    mark_bit(index, true);
    return vec2<u32>(index, level);
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

    // 🔴 One page per pixel is painted, and the sun wins when there is
    // one: a pixel is lit by many lights and painting the last one
    // walked would make the view depend on the light list's order.
    var painted = vec2<u32>(NO_PAGE, 0u);

    if pages.sun.w > 0.5 {
        painted = mark_sun(pages.sampling.y, world, wanted);
    }

    if view.dimensions.w == 0u {
        paint_page(pixel, painted);
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
        let marked = mark_local(light, world, wanted);
        if painted.x == NO_PAGE {
            painted = marked;
        }
    }

    paint_page(pixel, painted);
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

// Empties THIS VIEW'S entries, and only this view's (#866).
//
// 🔴 A pass instead of a `clear_buffer`, and the reason is the fused
// raster. `vbuf64.render` rasterises and shades in one fragment shader,
// so a view samples an atlas that is a frame old. Emptying the whole
// table once at the top of a frame would therefore leave whichever view
// marks SECOND reading a table the first one had just wiped — which is
// exactly the measured symptom: shadows in one viewport and none in the
// other.
//
// Keys are `page + 1` and a page carries its view in the high part, so
// ownership is a divide. No decode: the level and cell do not matter to
// a pass that only asks *whose is this*.
// Ages this view's table entries, and evicts the ones nothing has asked
// for in `max_age` frames.
//
// 🔴 This REPLACED `clear_view`, which emptied the whole table every
// frame. That is the change: a page nothing requested this frame is not
// gone, it is one frame older, and it stays resident — with its depth
// still in the atlas — until it has been unwanted for long enough to be
// worth the slot. Epic calls the threshold
// `MaxPageAgeSinceLastRequest`.
//
// It writes `PAGE_DEAD` rather than `PAGE_EMPTY`, which is not a detail:
// see the constant's own doc.
@compute @workgroup_size(MARK_GROUP * MARK_GROUP, 1, 1)
fn age_view(@builtin(global_invocation_id) id: vec3<u32>) {
    let entry = id.x;
    if entry >= pages.pool.x { return; }
    let key = atomicLoad(&table_keys[entry]);
    if key == PAGE_EMPTY || key == PAGE_DEAD { return; }
    if (key - 1u) / view_span() != pages.sampling.w { return; }

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
    atomicStore(&table_keys[entry], PAGE_DEAD);
    page_release(page_slot(entry));
    atomicAdd(&counters[12], 1u);
}
