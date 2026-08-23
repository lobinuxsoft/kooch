// page_table.wgsl — the virtual page id and where it lands (#866).
//
// CONCATENATED into every pass that touches the page table: the marking
// pass that WRITES it and the shading pass that READS it. This file
// holds what the two must agree on and nothing else.
//
// # A FLAT table, and why the hash it replaced is dead
//
// The lookup runs per pixel PER LIGHT in the shading pass, and prior
// art is unanimous that it must be ONE indexed read: Chalmers ("quite
// fast because they only require a single texture lookup"), Stephano's
// sparse VSM (`pageTable[ivec2(floor(uv * numPagesXY))]`, one
// indirection) and UE5 (`CalcPageOffset` is flat arithmetic over
// 21 845 entries per shadow map). This table hashed instead — open
// addressing with tombstones — and the measurement that killed it:
// shading 10.4 ms against 0.884 ms for the ENTIRE shadow track, on a
// walk of up to 5 chain levels times up to 32 probes, per pixel per
// light.
//
// The hash existed because the virtual space was 28 409 856 pages — a
// flat u32 each would be 108 MiB. Two decisions shrank the space to
// ~485 000 and the table to a few MiB, which is what made flat viable:
//
// - `LOCAL_MAX_TEXELS` caps a lamp's chain three levels below the
//   sun's, a factor of 64 in pages per lamp.
// - The address space stops PAYING for the capped levels: a lamp's
//   chain is addressed from `local_level_floor` up, so its stride is
//   2 046 pages instead of 131 070 — see `local_face_pages`.
//
// The entry's first word is `slot + 1`, 0 meaning "no page", so a
// cleared buffer is an empty table and no reset pass has to run.
// Eviction writes 0 — no tombstones, because nothing probes.
//
// # The VIEW is part of the key, and that is not a detail
//
// One editor frame draws the same world from two cameras. A clipmap is
// centred on ITS camera, so the same world position is a different page
// in each — and a table indexed without the view hands view B the pages
// view A marked. The symptom is exact and was measured: shadows in one
// viewport and none in the other. So the table is `views x span`
// entries and a page id carries its view in the high part.
//
// # The insert has no race, and that is not luck
//
// Only the thread that flipped a page's mark bit from 0 to 1 ever
// inserts it — `mark_bit` already returns exactly that. A key is
// claimed by one thread and the entry it writes is its own, so the
// stores are plain rather than compare-exchanges.

/// First word of an entry holding no page. Entries store `slot + 1`
/// so that a cleared buffer is an empty table; eviction stores this.
const PAGE_ABSENT: u32 = 0u;

/// Words per table entry: the physical slot (`slot + 1`, 0 = absent),
/// the frame it was last requested in, its index in this view's
/// compacted `page_list`, and the CONTENT STAMP — the generation the
/// page's atlas content was drawn under, `0` = no valid content.
///
/// # 🔴 The fourth word is what makes a cached page free
///
/// A resident page whose stamp equals its current generation (the
/// sun's per level — snapped centre and direction — or its lamp's —
/// transform, range, cone) keeps last frame's atlas content: the
/// compaction neither lists nor stamps it, the expansion never sees
/// it, and the depth pass no longer clears whole layers, only the
/// dirty pages' quads. StraySpark: *"cached pages are effectively
/// free"*; UE5 caches the same way (#477/#866).
///
/// Written by the compaction when it LISTS a page (drawn later the
/// same frame), zeroed by `page_stamp` when a fresh page claims the
/// entry and by `cs_invalidate` when a moved caster's sphere reaches
/// the page. Generations are never zero, so `0` always redraws.
///
/// 🔴 Interleaved because `max_storage_buffers_per_shader_stage` is
/// eight on the downlevel defaults; see the marking pass's binding.
const PAGE_CELL: u32 = 4u;

/// Culls a frame is willing to run for local lights — one per lamp,
/// the way the retired cube path ran one per face (#777). A lamp's
/// bucket is `chain.x + slot`, so a light past this cap has pages that
/// are listed and counted but never drawn, which the skipped counter
/// makes visible rather than silent.
///
/// # 🔴 Why a lamp cannot borrow the sun's survivors
///
/// A survivor list is a LOD picked for a VIEW. The sun's level-N list
/// is simplified for an orthographic box centred on the CAMERA — so a
/// lamp borrowing bucket N got geometry culled to someone else's
/// frustum (a close lamp's casters fell outside the fine levels' box
/// and its shadow vanished as it approached) at someone else's density
/// (a coarse bucket handed root meshlets, and a sphere's shadow was a
/// faceted lump). One cull per lamp, from the light's own eye with a
/// perspective error metric, is precisely the retired cube path's
/// recipe — the one path whose shadows were smooth.
///
/// Mirrors `LAMP_CULLS` in `pages/raster.rs`. 64 — twice the classic
/// path's `MAX_POINT_SHADOWS` — because the hierarchical cull (#939)
/// made a slot cheap: no per-lamp cull object, just a slice of the
/// shared arenas. The honest ceiling is the group-error arena,
/// `LAMP_CULLS × group_capacity × 4 B`.
const LAMP_CULLS: u32 = 64u;

/// Survivors one lamp may keep — its fixed slice of the shared
/// survivor arena, `[slot * LAMP_SURVIVORS ..)`. Fixed rather than
/// prefix-summed so the cull is one pass with no scan; a lamp past
/// its slice keeps a count larger than the cap, which is how the
/// panel sees the overflow. Mirrors `LAMP_SURVIVORS` in
/// `pages/lamp_cull.rs`.
const LAMP_SURVIVORS: u32 = 4096u;

/// A table entry that is resident but not in THIS view's `page_list`.
///
/// The third word is per view and the table is shared, so a page
/// belonging to another camera carries a listing from a compaction that
/// was not this one. Cleared to this rather than left stale.
const PAGE_UNLISTED: u32 = 0xffffffffu;

/// No physical page: either the pool is full or the probe gave up.
const PAGE_MISS: u32 = 0xffffffffu;

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
/// `span` is the pages one VIEW addresses, `stride` the pages one LOCAL
/// light addresses, `face_pages` one face's chain from the floor up,
/// `side` the pages across the sun's level 0. The sun's region sits
/// after the locals, at `sun_slot * stride`, and is `clipmap levels x
/// side^2` — nothing below needs its size because it is the tail.
///
/// 🔴 A local chain is addressed from `local_level_floor` UP. The
/// marking cannot pick a level below the floor, so addressing the
/// levels under it would spend table entries — most of the chain, the
/// fine levels are the wide ones — on pages that cannot exist. That is
/// the difference between a 131 070-page stride and a 2 046-page one,
/// and the flat table is only affordable with the second.
fn page_decode(
    page: u32,
    span: u32,
    stride: u32,
    face_pages: u32,
    side: u32,
    sun_slot: u32,
    page_texels: u32,
) -> PageId {
    var id: PageId;
    id.view = page / span;
    let within = page % span;
    let sun_base = sun_slot * stride;
    id.is_sun = within >= sun_base;

    if id.is_sun {
        // A clipmap's levels are all the same size, so the level is a
        // divide where a mip chain's is a walk.
        let per_level = side * side;
        id.light = sun_slot;
        id.face = 0u;
        let rest = within - sun_base;
        id.level = rest / per_level;
        let cell = rest % per_level;
        id.cell = vec2<u32>(cell % side, cell / side);
        return id;
    }

    id.light = within / stride;
    var rest = within % stride;
    id.face = rest / face_pages;
    rest = rest % face_pages;
    // The chain's levels are not the same size; walk it the way
    // `local_level_base` builds it — starting at the floor.
    var level = local_level_floor(side * page_texels);
    var wide = level_side_of(level, side);
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

/// The clipmap's centre for one level, SNAPPED to that level's page
/// grid.
///
/// # 🔴 This is what stops a shadow edge from crawling
///
/// A clipmap is centred on the camera, so without this every texel of it
/// slides through the world as the camera moves. A shadow edge is
/// decided per texel, so the edge is re-quantised every frame and the
/// whole silhouette shimmers — the classic shadow-map crawl, and the
/// reason it looks like the shadow is vibrating rather than moving.
///
/// Snapping the centre to whole pages pins the texels to world space:
/// the grid only jumps when the camera crosses a page, and a texel's
/// footprint never changes between jumps, so what it stores does not
/// change either. Every shipping implementation does this — it is not a
/// filter over the symptom, it removes the cause, and no amount of
/// temporal blending gets the same result without also smearing.
///
/// A page rather than a texel because a page is a whole number of
/// texels: aligning to it aligns both, and it also keeps a page's KEY
/// stable until the camera crosses a page, which a texel-grained snap
/// would not.
fn sun_centre(eye: vec3<f32>, basis: mat3x3<f32>, base: f32, side: u32, level: u32) -> vec2<f32> {
    let width = base * exp2(f32(level)) / f32(max(side, 1u));
    let plane = vec2<f32>(dot(eye, basis[0]), dot(eye, basis[1]));
    return floor(plane / width) * width;
}

/// A world position in the sun's plane, which is what every page lookup
/// is really indexing.
fn sun_plane(world: vec3<f32>, basis: mat3x3<f32>) -> vec2<f32> {
    return vec2<f32>(dot(world, basis[0]), dot(world, basis[1]));
}

/// The finest clipmap level whose extent still contains `reach`.
///
/// ⚠️ Carries slack for [`sun_centre`]: the snap moves the centre by up
/// to one page, so a point that fits a level measured from the camera
/// can fall outside the same level measured from the snapped grid. Two
/// pages of margin covers it, and costs a level only for points already
/// within a percent of the boundary.
fn sun_level(reach: f32, base: f32, side: u32) -> u32 {
    let slack = reach * (1.0 + 4.0 / f32(max(side, 1u)));
    if slack <= base {
        return 0u;
    }
    return u32(ceil(log2(max(slack / base, 1.0))));
}

/// Where one clipmap page sits in the sun's plane: `xy` its centre,
/// `z` its width. All three in metres, in the SUN'S PLANE — absolute,
/// not relative to the camera, because the grid the cell indexes is
/// snapped and the camera is not on it.
fn sun_page_rect(
    level: u32,
    cell: vec2<u32>,
    base: f32,
    side: u32,
    centre: vec2<f32>,
) -> vec3<f32> {
    let extent = base * exp2(f32(level));
    let width = extent / f32(side);
    // `mark_sun` maps the plane to `uv = (plane - centre) / extent +
    // 0.5`, so the cell's low corner is this and its centre is half a
    // page past it.
    let low = (vec2<f32>(cell) / f32(side) - vec2<f32>(0.5)) * extent + centre;
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
    // The sun's page is orthographic: `w` really is 1, so the local
    // position is already its own scaled form.
    return page_clip_w(local, depth, rect, atlas, 1.0);
}

/// The same, for a page whose projection has a `w`.
///
/// # 🔴 The sun's page has no `w` and a lamp's does
///
/// A clipmap page is ORTHOGRAPHIC: parallel rays, no foreshortening, and
/// `w = 1` is not a simplification but the truth. A lamp's page is a
/// perspective frustum from a point, so its `w` is the distance along
/// the face's major axis — and dividing by it at the vertex instead of
/// letting the rasteriser do it per fragment is not a rounding
/// difference. It is the difference between a projection and a
/// mapping.
///
/// Screen-space interpolation without a `w` is LINEAR. A triangle whose
/// vertices were each divided separately gets its interior filled by
/// straight lines between three correct points, which for the two large
/// triangles a floor is made of is wrong everywhere except the corners
/// — and wrong in a coherent, directional way that reads as every
/// shadow leaning the same direction.
fn page_clip_w(
    // The page-local position ALREADY multiplied by `w`, so nothing on
    // this path is ever divided before the rasteriser does it.
    local_w: vec2<f32>,
    depth_w: f32,
    rect: vec4<f32>,
    atlas: f32,
    w: f32,
) -> vec4<f32> {
    let half = rect.zw / atlas;
    let centre = (rect.xy + rect.zw * 0.5) / atlas * 2.0 - vec2<f32>(1.0);
    // Clip space is Y-up and a texel row is Y-down. The constant part
    // scales by `w` and the already-scaled part does not.
    let at = vec2<f32>(centre.x, -centre.y) * w
        + local_w * vec2<f32>(half.x, -half.y);
    return vec4<f32>(at, depth_w, w);
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
    // z the pool slots one view owns, w THIS FRAME's index.
    //
    // 🔴 `w` is read by one thing: the page age debug view, which paints
    // how many frames ago each page the reader lands on was last
    // requested. Nothing in the shading path depends on it.
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
    // x buckets in `page_list`, y the pair list's capacity, z pages one
    // bucket may list, w TRIANGLES A MESHLET MAY HOLD.
    //
    // 🔴 `x` is the clipmap's level count AND the bucket count, and that
    // is `page_octave`'s anchor rather than a coincidence: the sun's
    // level L is bucket L, so a lamp's pages land in buckets the sun's
    // culls already fill.
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

/// The direction a point on a cube face stands for — the inverse of
/// `cube_face`.
///
/// 🔴 Here, beside the forward map, for the reason the page arithmetic
/// is all in this file: an encoder and a decoder that drift produce
/// pages rasterised somewhere other than where they were asked for, and
/// nothing about the result says which half is wrong. A cube face's
/// axis conventions are six sign choices and every one of them is
/// invisible until a shadow lands on the wrong wall.
///
/// `uv` runs `[0, 1]` across the face, the way `cube_face` returns it.
/// The result is NOT normalised: it is the direction scaled so its major
/// axis is 1, which is what a face's own frustum wants.
fn face_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let t = uv * 2.0 - vec2<f32>(1.0);
    switch face {
        case 0u: { return vec3<f32>(1.0, -t.y, -t.x); }
        case 1u: { return vec3<f32>(-1.0, -t.y, t.x); }
        case 2u: { return vec3<f32>(t.x, 1.0, t.y); }
        case 3u: { return vec3<f32>(t.x, -1.0, -t.y); }
        case 4u: { return vec3<f32>(-t.x, -t.y, -1.0); }
        default: { return vec3<f32>(t.x, -t.y, 1.0); }
    }
}

/// The finest a LOCAL light's chain is allowed to go, in virtual texels
/// across one cube face.
///
/// # 🔴 A lamp does not get the sun's whole virtual map
///
/// The sun's clipmap is 16384 texels because it has to cover the world
/// and its finest level is centimetres from the camera. A four-metre
/// lamp asking for the same chain can request a HALF-MILLIMETRE texel,
/// and nobody looks at a shadow at that resolution — but the pool pays
/// for every page of it. Measured on `many_lights`: 455 of 504 resident
/// pages belonged to lamps and the sun was left forty-nine, with the
/// hash table walking nine tombstones per lookup because the pool never
/// stopped churning.
///
/// Epic caps this the same way and for the same reason —
/// `VSM_MAX_SINGLE_PAGE_SHADOW_MAPS` hands a distant light ONE page
/// rather than a chain — and exposes it as
/// `r.Shadow.Virtual.ResolutionLodBiasLocal`.
///
/// 2048 is three levels of the chain given up, which is a factor of
/// SIXTY-FOUR in the pages a lamp can address. The texel it leaves at
/// four metres is two millimetres.
const LOCAL_MAX_TEXELS: u32 = 2048u;

/// The finest chain level a local light may use.
///
/// Derived rather than uploaded: the marking picks a level, the reader
/// walks from one, and the raster sizes a cell from one — three places
/// that have to agree, and a number in a uniform is a number one of them
/// can be handed stale.
fn local_level_floor(virtual_texels: u32) -> u32 {
    var floor_level = 0u;
    var texels = virtual_texels;
    loop {
        if texels <= LOCAL_MAX_TEXELS || texels <= 1u {
            break;
        }
        texels = texels >> 1u;
        floor_level = floor_level + 1u;
    }
    return floor_level;
}

/// Pages across one side of a local chain's `level`. Mirrors
/// `PageConfig::side`, and takes the level-0 side rather than reading
/// the marking pass's uniform so the raster can call it too.
fn level_side_of(level: u32, side: u32) -> u32 {
    return max(side >> level, 1u);
}

/// Pages in one face's chain, from the floor up — a LOCAL light's
/// face stride.
///
/// Derived rather than uploaded for the same reason `local_level_floor`
/// is: the encoder, the decoder and the reader all need it, and a
/// number in a uniform is a number one of them can be handed stale.
fn local_face_pages(side: u32, page_texels: u32) -> u32 {
    var pages = 0u;
    var wide = level_side_of(local_level_floor(side * page_texels), side);
    loop {
        pages = pages + wide * wide;
        if wide == 1u {
            break;
        }
        wide = max(wide / 2u, 1u);
    }
    return pages;
}

/// Where `level` starts inside one face's chain, measured FROM THE
/// FLOOR. Mirrors `page_decode`'s walk: a mip chain's levels are not
/// the same size, so the offset is a running sum and not a multiply.
fn local_level_base(level: u32, side: u32, page_texels: u32) -> u32 {
    var base = 0u;
    var l = local_level_floor(side * page_texels);
    var wide = level_side_of(l, side);
    for (; l < level; l = l + 1u) {
        base = base + wide * wide;
        wide = max(wide / 2u, 1u);
    }
    return base;
}

/// A spot light's kind, as `GpuLight` stores it. Here because the
/// marking, the expansion, the depth raster and the reader all branch
/// on it, and a constant that drifts sends a spot's pages through a
/// point's projection.
const PAGE_KIND_SPOT: u32 = 2u;

/// A world offset from a SPOT light, rotated so the spot's axis is the
/// +X cube face — the one face `mark_local` assigns a spot.
///
/// # 🔴 A spot's face is ITS OWN axis, not the world's
///
/// `cube_face` is world-axis aligned. The first spot implementation
/// forced `face = 0` while keeping the world-axis uv, and the depth
/// raster projected through the world's +X — three different mappings
/// of the same page, and the measured result was occlusion the shape
/// of nothing that exists. Rotating the offset FIRST makes face 0 the
/// natural answer for every point inside a cone up to 90 degrees, and
/// every pass then agrees by construction.
///
/// Shared here for the same reason `sun_basis` is: the writer, the
/// raster and the reader must build the SAME basis, or a page is
/// rasterised somewhere other than where it is read.
fn spot_local(direction: vec3<f32>, offset: vec3<f32>) -> vec3<f32> {
    let d = normalize(direction);
    var up = vec3<f32>(0.0, 1.0, 0.0);
    if abs(d.y) > 0.99 {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    let s = normalize(cross(d, up));
    let u = cross(s, d);
    return vec3<f32>(dot(offset, d), dot(offset, u), dot(offset, s));
}

/// The near plane every local page is rasterised with.
///
/// 🔴 Shared with `SPOT_SHADOW_NEAR_Z` and the cube pass by value, not
/// by import — the reader reconstructs depth as `near / distance`, so a
/// near the writer and the reader disagree on is a depth comparison that
/// is wrong by a constant factor everywhere.
const PAGE_NEAR: f32 = 0.05;

/// Where a world offset from a lamp lands inside ONE cell of ONE face,
/// in that cell's own clip space.
///
/// `xy` runs `[-1, 1]` across the cell and `z` is the distance along the
/// face's major axis — negative or zero means the point is behind the
/// face and belongs to another one.
///
/// The cell is a sub-rect of the face, so this is the face's own
/// projection with the cell's rect mapped back out to full clip: the
/// same narrowing `page_clip` does in texels, done in angle.
/// A world offset from a lamp, rotated into ONE cube face's own space:
/// `xy` across the face and `z` along its axis, positive in front.
///
/// 🔴 The inverse of the face selection, applied UNCONDITIONALLY. It
/// does not ask which face the offset belongs to — a point behind this
/// face simply comes back with a negative `z`, which is what a
/// projection's `w` is for.
///
/// That distinction is the whole reason this exists. Asking the face and
/// rejecting a mismatch works per POINT and a triangle has three of
/// them: a triangle straddling a seam had one vertex pushed outside the
/// clip volume while the other two projected normally, and the clipper
/// interpolated between them — producing a wedge of geometry along
/// every seam, rasterised into a page it never touched. On screen that
/// is a straight bar of false occlusion crossing the lamp's pool.
fn face_local(face: u32, offset: vec3<f32>) -> vec3<f32> {
    switch face {
        case 0u: { return vec3<f32>(-offset.z, -offset.y, offset.x); }
        case 1u: { return vec3<f32>(offset.z, -offset.y, -offset.x); }
        case 2u: { return vec3<f32>(offset.x, offset.z, offset.y); }
        case 3u: { return vec3<f32>(offset.x, -offset.z, -offset.y); }
        case 4u: { return vec3<f32>(-offset.x, -offset.y, -offset.z); }
        default: { return vec3<f32>(offset.x, -offset.y, offset.z); }
    }
}

/// Where a world offset from a lamp lands in ONE cell of ONE face, in
/// that cell's clip space — `xy` ALREADY multiplied by `w`, and `z` the
/// `w` itself.
///
/// Handing back the undivided form is the point: the rasteriser divides
/// per fragment, and a vertex shader that divides first fills a triangle
/// with straight lines between three separately-divided corners.
fn cell_face(face: u32, cell: vec2<u32>, side: u32, offset: vec3<f32>) -> vec3<f32> {
    let local = face_local(face, offset);
    let step = 1.0 / f32(max(side, 1u));
    let low = vec2<f32>(cell) * step;
    // `uv = local.xy / local.z * 0.5 + 0.5`, then `(uv - low) / step`
    // mapped to `[-1, 1]` — all of it multiplied through by `local.z`
    // so nothing is divided here.
    let scaled = (local.xy * 0.5 + local.z * (vec2<f32>(0.5) - low)) / step;
    return vec3<f32>(scaled * 2.0 - vec2<f32>(local.z), local.z);
}

/// Whether a sphere can reach the cell of a cube face a local page
/// stands for.
///
/// # 🔴 A cone, not a box
///
/// The sun's page is a slab: parallel sides, one width, and a sphere
/// against it is two absolute values. A lamp's page is a FRUSTUM from a
/// point — it gets wider with distance and it has no width of its own —
/// so the same test against a box is wrong at every distance except the
/// one the box was built at.
///
/// The cell's circumscribing cone is conservative: it covers the square
/// cell plus the corners, so a meshlet that only clips a corner is
/// admitted and rejected later by the raster's own clip. Over-emitting a
/// pair costs a rasterised triangle that discards; under-emitting costs
/// a missing shadow with nothing to say why.
///
/// `axis` is the cell's centre direction and `cos_half` the cosine of
/// the angle from it to the cell's corner — both from `face_dir`, so the
/// cell this covers is the cell the marking assigned.
fn cell_reaches(
    axis: vec3<f32>,
    cos_half: f32,
    to_centre: vec3<f32>,
    radius: f32,
    range: f32,
) -> bool {
    let distance = length(to_centre);
    // Past the lamp's reach entirely, and the near case where the
    // sphere swallows the apex: every direction is inside it.
    if distance > range + radius {
        return false;
    }
    if distance <= radius {
        return true;
    }
    // The sphere subtends `asin(radius / distance)` from the apex, so
    // it reaches the cone when the angle between them is under the sum.
    // Compared as cosines to keep it to one `acos` per test rather than
    // two.
    let cos_to = dot(to_centre / distance, axis);
    let angle = acos(clamp(cos_to, -1.0, 1.0));
    let half = acos(clamp(cos_half, -1.0, 1.0));
    return angle <= half + asin(clamp(radius / distance, 0.0, 1.0));
}

/// The cell's centre direction and the cosine of its corner half-angle.
///
/// `xyz` the axis, `w` the cosine — one call because both come from the
/// same four `face_dir` evaluations and a caller that recomputed them
/// separately is a caller that can disagree with itself.
fn cell_cone(face: u32, cell: vec2<u32>, side: u32) -> vec4<f32> {
    let step = 1.0 / f32(max(side, 1u));
    let low = vec2<f32>(cell) * step;
    let axis = normalize(face_dir(face, low + vec2<f32>(step * 0.5)));
    // The corner furthest from the axis. A face's mapping is not
    // angle-linear, so the four corners are not equidistant and the
    // smallest cosine is the one that bounds them all.
    var cos_half = 1.0;
    for (var i = 0u; i < 4u; i = i + 1u) {
        let corner = low + vec2<f32>(f32(i & 1u), f32(i >> 1u)) * step;
        cos_half = min(cos_half, dot(normalize(face_dir(face, corner)), axis));
    }
    return vec4<f32>(axis, cos_half);
}

/// What one texel of this page covers, in metres.
///
/// The two chains measure it differently and both are exact. The sun's
/// clipmap level spans a known extent, so a texel is that over the
/// virtual texels across it. A local light's face is a 90-degree
/// perspective, so at the light's range it covers `2 * range` and a
/// texel is that over the level's texel count — the same identity
/// `page_level` inverts when the marking picks the level.
fn page_texel_world(
    id: PageId,
    base: f32,
    virtual_texels: u32,
    range: f32,
) -> f32 {
    if id.is_sun {
        return base * exp2(f32(id.level)) / f32(max(virtual_texels, 1u));
    }
    return 2.0 * range / f32(max(virtual_texels >> id.level, 1u));
}

/// Which bucket of `page_list` a page belongs in: an OCTAVE of world
/// texel size.
///
/// # 🔴 A bucket is a density, not a light and not a chain
///
/// The expansion pairs a bucket's pages against a bucket's surviving
/// meshlets, so what a bucket has to mean is "everything that wants
/// geometry at this fineness". A lamp two metres from a wall and the sun
/// forty metres out can want the same texel size, and when they do they
/// want the same LOD — so they belong in the same list. Bucketing by
/// chain level instead puts them in different ones and needs a cull per
/// light to fill the second, which is the cost that grows with the
/// scene.
///
/// The scale is anchored so the sun's clipmap level `L` lands on bucket
/// `L` exactly: its texel is `base * 2^L / virtual`, and the finest is
/// `base / virtual`, so the ratio IS `2^L`. That is what lets a local
/// light's pages fall into buckets the sun's culls already fill —
/// without one new dispatch.
///
/// ⚠️ Clamped at both ends. A lamp finer than the sun's level 0 draws
/// from level 0's survivors, which is geometry finer than it needs
/// rather than coarser — the safe direction.
fn page_octave(texel: f32, base: f32, virtual_texels: u32, levels: u32) -> u32 {
    let finest = base / f32(max(virtual_texels, 1u));
    // 🔴 The nudge is the anchor holding. The sun's ratio is EXACTLY
    // `2^L` in arithmetic and only exactly `2^L` in floating point when
    // `base` happens to be a power of two — the engine's is 1.28. One
    // ulp low and `log2` returns `L - tiny`, `floor` returns `L - 1`,
    // and the level draws a coarser level's survivors: geometry at the
    // wrong LOD, in the sun's own pages, for every clipmap level whose
    // division rounded down.
    //
    // Octaves are a whole apart, so 1e-4 cannot move a decision that
    // was not already a rounding accident.
    let octave = floor(log2(max(texel, 1e-9) / max(finest, 1e-9)) + 1e-4);
    return u32(clamp(octave, 0.0, f32(max(levels, 1u) - 1u)));
}

