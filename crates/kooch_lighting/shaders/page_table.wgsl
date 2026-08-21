// page_table.wgsl — the virtual page id and where it lands (#866).
//
// CONCATENATED into every pass that touches the page table: the marking
// pass that WRITES it and, later, the shading pass that READS it. This
// file holds what the two must agree on and nothing else.
//
// # Why a hash, and why the obvious answer is dead
//
// With 128-texel pages over a 16384 virtual map, a mip chain per cube
// face and a 17-level clipmap, one light addresses 278 528 pages. A
// hundred lights and a sun make the virtual space **28 409 856 pages**.
//
// - The MARK bitmap is one bit each: 3.4 MiB. Affordable, and that is
//   why marking was built first.
// - A FLAT table is one `u32` each: **108 MiB, 42 % of the 256 MiB pool
//   it indexes**, to describe pages that are 99.99 % empty. Dead on
//   arrival, and it also kills the obvious allocator — a sweep over the
//   virtual space is a 28-million-thread dispatch to find ~2000 set
//   bits.
// - A HIERARCHICAL table is small, but it pays an indirection per
//   lookup, and the lookup is per pixel per light in the shading pass.
//   That is the hot path the froxel grid exists to keep short.
//
// So the table is sized to what is RESIDENT, not to what is
// addressable: open addressing over `2 x pool_pages` entries, which for
// Epic's 4096-page pool is 8192 slots — **64 KiB**, and one probe in the
// common case.
//
// ⚠️ **UE5 does NOT hash it**, and an earlier version of this comment
// said it did. `CalcPageOffset` in `VirtualShadowMapPageAccessCommon.ush`
// is flat arithmetic — `id * VSM_PAGE_TABLE_SIZE + level_offset + x + y
// * dims` — over 21 845 entries per shadow map. Epic pays 87 KiB per map
// and stays small by never handing a distant light a full virtual space:
// `VSM_MAX_SINGLE_PAGE_SHADOW_MAPS` is 8192 maps of ONE entry each. The
// 108 MiB above is what a flat table costs *given our decision to give
// every light the full space*, which is a decision, not a law.
//
// # The VIEW is part of the key, and that is not a detail
//
// One editor frame draws the same world from two cameras. A clipmap is
// centred on ITS camera, so the same world position is a different page
// in each — and a table keyed without the view hands view B the pages
// view A marked. The symptom is exact and was measured: shadows in one
// viewport and none in the other.
//
// UE5's answer is the `VirtualShadowMapId`: every (view, light, clipmap
// level) triple gets its own id and the id IS the high part of the page
// address. This does the same with a multiply — `view * view_span` —
// because the hash makes the size of the address space free.
//
// # The insert has no race, and that is not luck
//
// Only the thread that flipped a page's mark bit from 0 to 1 ever
// inserts it — `mark_bit` already returns exactly that. So a key is
// claimed by one thread, and the compare-exchange below is there for
// DIFFERENT keys landing on the same slot, never for two threads
// fighting over one page. That is what makes the physical index safe to
// write with a plain store right after.

// 0 is EMPTY, so a cleared buffer is an empty table and no reset pass
// has to run. Keys are therefore stored as `page + 1`.
const PAGE_EMPTY: u32 = 0u;

/// An entry whose page was EVICTED, and the reason persistence needs a
/// third state at all.
///
/// 🔴 Open addressing resolves a collision by walking, so a lookup stops
/// at the first EMPTY entry: an empty slot proves the key was never
/// inserted, because inserting it would have taken that slot. Writing
/// EMPTY over an evicted key breaks that proof — every key whose probe
/// run passed through the freed entry becomes unfindable while still
/// sitting in the table, and the symptom is a page that is resident,
/// rasterised, and never sampled.
///
/// A tombstone keeps the run intact. Readers walk past it; an insert may
/// reuse it. This costs a probe per hole, which `counters[9]` counts so
/// that a table degrading into holes is a number rather than a mystery.
const PAGE_DEAD: u32 = 0xfffffffeu;

/// Words per table entry: the physical slot, then the frame it was last
/// requested in.
///
/// 🔴 Interleaved because `max_storage_buffers_per_shader_stage` is
/// eight on the downlevel defaults and the marking pass was already
/// there. Declared here rather than in the marking pass because the
/// SHADING pass indexes the same buffer and a stride the two disagree on
/// reads an age as a slot.
const PAGE_CELL: u32 = 2u;

/// No physical page: either the pool is full or the probe gave up.
const PAGE_MISS: u32 = 0xffffffffu;

/// How far a lookup walks before calling it a miss.
///
/// At a load factor of 0.5 the expected probe count is under 2; 32 is
/// the point where something is wrong with the hash rather than with
/// the load.
const PAGE_PROBES: u32 = 32u;

/// Murmur3's finalizer. Any bijection on 32 bits would do; what matters
/// is that page indices are DENSE and highly structured — consecutive
/// ids differ in the low bits and share every high one — so the low bits
/// alone would pile every page of a level onto one run of slots.
fn page_hash(key: u32) -> u32 {
    var h = key;
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return h;
}

/// Where a key's probe sequence starts. `entries` is a power of two.
fn page_probe(key: u32, entries: u32) -> u32 {
    return page_hash(key + 1u) & (entries - 1u);
}

/// The next slot in the sequence. Linear, because at load factor 0.5 the
/// clustering costs less than the cache misses a smarter sequence buys.
fn page_step(probe: u32, entries: u32) -> u32 {
    return (probe + 1u) & (entries - 1u);
}

/// The texel a physical page starts at, inside its layer.
///
/// The atlas is a plain grid of pages. `per_row` is a constant of the
/// pool, not of the light, so nothing about a page's ADDRESS survives
/// into its CONTENT — which is what lets a page be evicted and refilled
/// without anything that samples it noticing.
fn page_origin(slot: u32, per_row: u32, page: u32) -> vec2<u32> {
    return vec2<u32>(slot % per_row, slot / per_row) * page;
}

/// A physical slot taken apart: `xy` the page's origin in texels inside
/// its layer, `z` the layer.
///
/// 🔴 The atlas is an array with a LAYER PER VIEW, so a view renders
/// into its own attachment and clears it without a scissor, a stencil
/// or a clearing draw — the three ways a shared surface is normally
/// partitioned, all of which this avoids. Slots stay GLOBAL so a table
/// entry says where its page lives without being told whose it is.
fn page_place(slot: u32, slice: u32, per_row: u32, page: u32) -> vec3<u32> {
    let width = max(slice, 1u);
    return vec3<u32>(page_origin(slot % width, per_row, page), slot / width);
}

// ---------------------------------------------------------------------
// Decoding a virtual page, and the geometry it stands for.
//
// The MARKING pass encodes these indices and the RASTER decodes them.
// Both live here for the same reason the hash does: an encoder and a
// decoder that drift produce pages rasterised somewhere other than where
// they were asked for, and nothing about that failure says which half is
// wrong.
// ---------------------------------------------------------------------

/// A virtual page taken apart. `light` is the sun's slot when
/// `is_sun` is true, in which case `face` is meaningless.
struct PageId {
    /// Which camera asked for it. Two viewports over one world are two
    /// clipmaps, and a page of one is not a page of the other.
    view: u32,
    light: u32,
    face: u32,
    level: u32,
    cell: vec2<u32>,
    is_sun: bool,
}

/// Inverts the arithmetic in `mark_local` and `mark_sun`.
///
/// `span` is the pages one VIEW addresses, `stride` the pages one light
/// addresses, `face_pages` one face's whole mip chain, `side` the pages
/// across level 0.
fn page_decode(
    page: u32,
    span: u32,
    stride: u32,
    face_pages: u32,
    side: u32,
    sun_slot: u32,
) -> PageId {
    var id: PageId;
    id.view = page / span;
    let within = page % span;
    id.light = within / stride;
    var rest = within % stride;
    id.is_sun = id.light == sun_slot;

    if id.is_sun {
        // A clipmap's levels are all the same size, so the level is a
        // divide where a mip chain's is a walk.
        let per_level = side * side;
        id.face = 0u;
        id.level = rest / per_level;
        let cell = rest % per_level;
        id.cell = vec2<u32>(cell % side, cell / side);
        return id;
    }

    id.face = rest / face_pages;
    rest = rest % face_pages;
    // The chain's levels are not the same size; walk it the way
    // `level_base` builds it.
    var level = 0u;
    var wide = side;
    loop {
        let count = wide * wide;
        if rest < count || wide == 1u {
            break;
        }
        rest = rest - count;
        wide = max(wide / 2u, 1u);
        level = level + 1u;
    }
    id.level = level;
    id.cell = vec2<u32>(rest % wide, rest / wide);
    return id;
}

/// The sun's basis. Built rather than uploaded, and built the SAME way
/// in both passes: the sun has no position, so this is the only place
/// its orientation means anything, and a second copy free to pick a
/// different `up` would rasterise into pages nobody marked.
fn sun_basis(direction: vec3<f32>) -> mat3x3<f32> {
    let f = normalize(direction);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(f.y) > 0.99 {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    return mat3x3<f32>(s, u, f);
}

/// Where one clipmap page sits in the sun's plane: `xy` its centre,
/// `z` its width. All three in metres, relative to the camera.
fn sun_page_rect(level: u32, cell: vec2<u32>, base: f32, side: u32) -> vec3<f32> {
    let extent = base * exp2(f32(level));
    let width = extent / f32(side);
    // `mark_sun` maps the plane to `uv = plane / extent + 0.5`, so the
    // cell's low corner is this and the centre is half a page past it.
    let low = (vec2<f32>(cell) / f32(side) - vec2<f32>(0.5)) * extent;
    return vec3<f32>(low + vec2<f32>(width * 0.5), width);
}

/// A page's rect inside its atlas layer, in texels: `xy` the origin,
/// `zw` the size.
fn page_atlas_rect(slot: u32, slice: u32, per_row: u32, page: u32) -> vec4<f32> {
    let origin = vec2<f32>(page_place(slot, slice, per_row, page).xy);
    return vec4<f32>(origin, vec2<f32>(f32(page)));
}

/// Places a page's own clip position inside the atlas.
///
/// The atlas is ONE render target and every page is a sub-rect of it, so
/// the draw is one pass rather than one pass per page — the difference
/// between 17 render passes and 1681 of them.
///
/// ⚠️ It does NOT clip. A triangle wider than its page still rasterises
/// past the rect and into a neighbour that belongs to another level or
/// another light. The fragment shader is what stops that, and it is the
/// reason this pipeline has one at all.
fn page_clip(local: vec2<f32>, depth: f32, rect: vec4<f32>, atlas: f32) -> vec4<f32> {
    let half = rect.zw / atlas;
    let centre = (rect.xy + rect.zw * 0.5) / atlas * 2.0 - vec2<f32>(1.0);
    // Clip space is Y-up and a texel row is Y-down.
    let at = vec2<f32>(centre.x, -centre.y) + local * vec2<f32>(half.x, -half.y);
    return vec4<f32>(at, depth, 1.0);
}

// ---------------------------------------------------------------------
// What every raster pass needs to know. One declaration for the three of
// them — compaction, expansion and the draw — because they walk the same
// page ids and a field that means one thing in one pass and another in
// the next is a page rasterised into someone else's rect.
// ---------------------------------------------------------------------

struct PageRaster {
    // x the per-light stride in pages, y one face's whole chain,
    // z pages across level 0, w the sun's slot WITHIN a view.
    space: vec4<u32>,
    // x the view these pages belong to, y the pages one view addresses,
    // z the pool slots one view owns, w unused.
    //
    // 🔴 The pool is SLICED, not shared, and the slice is what lets a
    // view empty and refill its own pages without touching the other
    // view's — which is what the other view is still reading, because
    // raster and shading are fused and the atlas it samples is a frame
    // old.
    views: vec4<u32>,
    // x table entries, y physical pool pages, z pages across the atlas,
    // w page texels.
    pool: vec4<u32>,
    // x levels in the clipmap, y the pair list's capacity, z pages one
    // level may list, w TRIANGLES A MESHLET MAY HOLD.
    //
    // 🔴 `w` is the fixed vertex count the indirect draw issues, over
    // three. It is the builder's `max_triangles_per_meshlet` and NOT
    // the meshlet count of a mesh — confusing the two issues about a
    // third of the vertices a meshlet needs, which cuts every meshlet
    // short and turns a shadow into fragments that follow the meshlet
    // structure.
    chain: vec4<u32>,
    // x the clipmap's level-0 extent in metres, y the orthographic half
    // span, z the atlas side in texels, w unused.
    world: vec4<f32>,
    // xyz the camera, w unused.
    eye: vec4<f32>,
    // xyz the sun's direction, w 1 when there is one.
    sun: vec4<f32>,
}

