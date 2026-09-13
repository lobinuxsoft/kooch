// Virtual page ids (#866), shared by marking and shading. Flat, one read per pixel per light
// (Chalmers, UE5); the old hash cost 10.4 ms. Entries are `slot + 1`, the view is in the key, and
// only a page's marker inserts it.

/// First word of an entry holding no page. Entries store `slot + 1`
/// so that a cleared buffer is an empty table; eviction stores this.
const PAGE_ABSENT: u32 = 0u;

/// Words: slot + 1, frame requested, `page_list` index, content stamp (0 = none), `PAGE_LOD`, frame
/// allocated. 🔴 A matching stamp keeps last frame's content for free. Interleaved: downlevel allows
/// eight storage buffers.
const PAGE_CELL: u32 = 6u;

/// Levels up to the first readable page, or `PAGE_NO_LOD`: a jump instead of up to 17 misses
/// (Unreal's `LODOffset`). 🔴 In the receiver bound's old word — per-level bounds lost casters to
/// climbing readers.
const PAGE_LOD: u32 = 4u;

/// No coarser level holds a readable page for this position.
const PAGE_NO_LOD: u32 = 0xffffffffu;

/// Stamp for a cleared page, valid under every generation. 🔴 Even: generations end `h | 1`, so none
/// collides; `0` already means redraw.
const PAGE_EMPTY: u32 = 2u;

/// Lamp culls per frame, one per lamp in bucket `chain.x + slot`. 🔴 Lamps can't borrow the sun's
/// camera-box survivors. 256 mirrors `pages/raster.rs`: `many_lights` casts ~100, 64 lost 121
/// pages.
const LAMP_CULLS: u32 = 256u;

/// Survivors per lamp, a fixed slice `[slot * LAMP_SURVIVORS ..)` so the cull needs no scan; an
/// over-cap count shows the overflow. Mirrors `pages/lamp_cull.rs`.
const LAMP_SURVIVORS: u32 = 4096u;

/// Resident but not in THIS view's `page_list`: the listing word is per view and the table is
/// shared.
const PAGE_UNLISTED: u32 = 0xffffffffu;

/// No physical page: either the pool is full or the probe gave up.
const PAGE_MISS: u32 = 0xffffffffu;

/// A slot's texel origin in its layer. `per_row` belongs to the pool, so a page can be evicted and
/// refilled unnoticed.
fn page_origin(slot: u32, per_row: u32, page: u32) -> vec2<u32> {
    return vec2<u32>(slot % per_row, slot / per_row) * page;
}

/// The layer a slot lives in. 🔴 A rect is the same texels on every layer, so a draw in the wrong
/// layer overwrites another page (#1016).
fn page_layer(slot: u32, slice: u32) -> u32 {
    return slot / max(slice, 1u);
}

/// A slot as `xy` origin and `z` layer. 🔴 A layer per view: each view clears its own attachment
/// with no scissor or stencil; slots stay global.
fn page_place(slot: u32, slice: u32, per_row: u32, page: u32) -> vec3<u32> {
    let width = max(slice, 1u);
    return vec3<u32>(page_origin(slot % width, per_row, page), slot / width);
}

// Decoding a virtual page: the marking encodes, the raster decodes, both here so they cannot drift.

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

/// Inverts `mark_local` and `mark_sun`: locals first at `stride` pages each, then the sun's `levels
/// × side²` tail. 🔴 Local chains are addressed from `local_level_floor` up — 2 046 pages a stride,
/// not 131 070.
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

/// The sun's basis, built identically in both passes: a second copy with another `up` rasterises
/// pages nobody marked.
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

/// Receiver depth gradient per texel per axis: a tilt crosses depth along one axis only, so a
/// scalar bias detaches the other. `-n.xy / n.z` in the sun frame; ⚠️ `limit` is required, it
/// diverges edge-on.
fn receiver_slope(
    normal: vec3<f32>,
    basis: mat3x3<f32>,
    texel_world: f32,
    span: f32,
    limit: f32,
) -> vec2<f32> {
    let n = vec3<f32>(
        dot(normal, basis[0]),
        dot(normal, basis[1]),
        dot(normal, basis[2]),
    );
    // Magnitude floored, sign kept: `n.z` passes through zero exactly
    // when the surface turns edge-on, and both sides of it are real
    // surfaces that still have to be shaded.
    let size = max(abs(n.z), 1e-4);
    let denom = select(-size, size, n.z >= 0.0);
    let ratio = clamp(n.xy / denom, vec2<f32>(-limit), vec2<f32>(limit));
    // Into the depth encoding: `receiver` falls as `along` grows over
    // `2 * span`, and the plane's own fall cancels that sign.
    return ratio * (texel_world / (2.0 * max(span, 1e-6)));
}

/// The clipmap centre per level, snapped to its page grid. 🔴 Stops shadow-edge crawl: texels pinned
/// to the world only jump when the camera crosses a page, and page keys stay stable.
fn sun_centre(eye: vec3<f32>, basis: mat3x3<f32>, base: f32, side: u32, level: u32) -> vec2<f32> {
    let width = base * exp2(f32(level)) / f32(max(side, 1u));
    let plane = vec2<f32>(dot(eye, basis[0]), dot(eye, basis[1]));
    return floor(plane / width) * width;
}

/// Camera offset from the level's snapped depth origin: raw-camera depth invalidated every page per
/// move, 29.7 vs 0.064 ms (#948). ⚠️ `floor(x + 0.5)`: WGSL and Rust round halves differently.
fn sun_drift(eye: vec3<f32>, basis: mat3x3<f32>, base: f32, side: u32, level: u32) -> f32 {
    let width = base * exp2(f32(level)) / f32(max(side, 1u));
    let along = dot(eye, basis[2]);
    return along - floor(along / width + 0.5) * width;
}

/// A world position in the sun's plane, which is what every page lookup
/// is really indexing.
fn sun_plane(world: vec3<f32>, basis: mat3x3<f32>) -> vec2<f32> {
    return vec2<f32>(dot(world, basis[0]), dot(world, basis[1]));
}

/// Finest level containing `reach`, with two pages of slack for [`sun_centre`]'s snap.
fn sun_level(reach: f32, base: f32, side: u32) -> u32 {
    let slack = reach * (1.0 + 4.0 / f32(max(side, 1u)));
    if slack <= base {
        return 0u;
    }
    return u32(ceil(log2(max(slack / base, 1.0))));
}

/// Floored modulo: WGSL's `%` follows the dividend, and half the world has negative page indices.
fn wrap_to(v: vec2<f32>, m: f32) -> vec2<f32> {
    return v - floor(v / m) * m;
}

/// Lowest absolute page index in this level's window. 🔴 From the eye, never `floor(centre /
/// width)`: the f32 round trip lands a page low for 7.2% of positions, and the jitter killed
/// caching (flat 27 ms).
fn sun_window(
    eye: vec3<f32>,
    basis: mat3x3<f32>,
    base: f32,
    side: u32,
    level: u32,
) -> vec2<f32> {
    let s = f32(max(side, 1u));
    let width = base * exp2(f32(level)) / s;
    return floor(sun_plane(eye, basis) / width) - vec2<f32>(floor(s * 0.5));
}

/// A point's page: its absolute world index wrapped into `side × side`. 🔴 Toroidal: keying by
/// camera offset shifted every page when the camera crossed one — 72 FPS still, 5 moving on the
/// OneXFly (#948).
fn sun_cell(
    world: vec3<f32>,
    eye: vec3<f32>,
    basis: mat3x3<f32>,
    base: f32,
    side: u32,
    level: u32,
) -> vec2<u32> {
    let s = f32(max(side, 1u));
    let width = base * exp2(f32(level)) / s;
    let low = sun_window(eye, basis, base, side, level);
    // ⚠️ Clamped into the window before wrapping, or an outside index aliases onto a page
    // elsewhere.
    let idx = clamp(
        floor(sun_plane(world, basis) / width),
        low,
        low + vec2<f32>(s - 1.0),
    );
    return vec2<u32>(wrap_to(idx, s));
}

/// The absolute world index a wrapped cell stands for — a page's real identity.
fn sun_page_index(
    level: u32,
    cell: vec2<u32>,
    eye: vec3<f32>,
    basis: mat3x3<f32>,
    base: f32,
    side: u32,
) -> vec2<f32> {
    let s = f32(max(side, 1u));
    let low = sun_window(eye, basis, base, side, level);
    return low + wrap_to(vec2<f32>(cell) - low, s);
}

/// One clipmap page in the sun's plane: `xy` centre, `z` width, in absolute metres, since the grid
/// is snapped and the camera is not.
fn sun_page_rect(
    level: u32,
    cell: vec2<u32>,
    eye: vec3<f32>,
    basis: mat3x3<f32>,
    base: f32,
    side: u32,
) -> vec3<f32> {
    let width = base * exp2(f32(level)) / f32(max(side, 1u));
    let idx = sun_page_index(level, cell, eye, basis, base, side);
    return vec3<f32>(idx * width + vec2<f32>(width * 0.5), width);
}

/// One FNV-1a round, for folding a page's identity into a generation.
/// Mirrors `fnv` in `pages/raster.rs`.
fn page_mix(h: u32, v: u32) -> u32 {
    return (h ^ v) * 0x01000193u;
}

/// A page's rect inside its atlas layer, in texels: `xy` the origin,
/// `zw` the size.
fn page_atlas_rect(slot: u32, slice: u32, per_row: u32, page: u32) -> vec4<f32> {
    let origin = vec2<f32>(page_place(slot, slice, per_row, page).xy);
    return vec4<f32>(origin, vec2<f32>(f32(page)));
}

/// A page's clip position inside the one-target atlas: one pass, not 1681. ⚠️ No clipping — the
/// fragment shader stops triangles bleeding into neighbours.
fn page_clip(local: vec2<f32>, depth: f32, rect: vec4<f32>, atlas: f32) -> vec4<f32> {
    // The sun's page is orthographic: `w` really is 1, so the local
    // position is already its own scaled form.
    return page_clip_w(local, depth, rect, atlas, 1.0);
}

/// The same for a page with a `w`. 🔴 Sun pages are orthographic (w = 1); a lamp's is a frustum, and
/// dividing at the vertex interpolates linearly, leaning every shadow.
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

// What every raster pass needs — compaction, expansion, draw — declared once since they walk the
// same page ids.

struct PageRaster {
    // x the per-light stride in pages, y one face's whole chain,
    // z pages across level 0, w the sun's slot WITHIN a view.
    space: vec4<u32>,
    // x view, y pages per view, z pool slots per view, w frame index (only the page age view reads
    // it). 🔴 The pool is sliced per view, so a view refills its pages without touching the other's.
    views: vec4<u32>,
    // x table entries, y physical pool pages, z pages across the atlas,
    // w page texels.
    pool: vec4<u32>,
    // x buckets (= clipmap levels, `page_octave`'s anchor), y pair capacity, z pages per bucket, w
    // triangles per meshlet. 🔴 `w` is `max_triangles_per_meshlet`, not a meshlet count — confusing
    // them cuts meshlets short.
    chain: vec4<u32>,
    // x the clipmap's level-0 extent in metres, y the orthographic half
    // span, z the atlas side in texels, w the PCF box width in texels.
    world: vec4<f32>,
    // xyz the camera, w unused.
    eye: vec4<f32>,
    // xyz the sun's direction, w 1 when there is one.
    sun: vec4<f32>,
    // x normal step in texels, y step toward the light in metres, z cap in metres (0 = none). 🔴
    // Texels span five orders of magnitude across the chain; without `z` the step reaches 9.2 m.
    bias: vec4<f32>,
    // x the attached layer, y its view. 🔴 A page whose slot is in another layer must not draw —
    // that is corruption, not a miss (#1016).
    layer: vec4<u32>,
}

// The cube face a direction lands on and its position across it, as `face_view_proj` without the
// matrix.
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

/// The direction a point on a face stands for — `cube_face`'s inverse, kept beside it since six
/// sign choices are invisible until a shadow hits the wrong wall. `uv` in [0, 1]; unnormalised,
/// major axis 1.
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

/// Finest texels per face for lamps: on the sun's 16384 a 4 m lamp wants 0.5 mm, and lamps held 455
/// of 504 pages. 2048 drops three levels (64× fewer pages), 2 mm at 4 m — Epic caps it too.
const LOCAL_MAX_TEXELS: u32 = 2048u;

/// Finest level a lamp may use — derived, not uploaded, since marking, reader and raster must
/// agree.
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

/// Pages in one face's chain from the floor up, a lamp's face stride; derived for the same reason.
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

/// A spot's kind as `GpuLight` stores it; marking, expansion, raster and reader all branch on it.
const PAGE_KIND_SPOT: u32 = 2u;

/// A spot offset rotated so its axis is the +X face `mark_local` assigns. 🔴 The spot's own axis,
/// not the world's — mixed mappings gave occlusion shaped like nothing. Shared so writer, raster
/// and reader agree.
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

/// Near plane for local pages. 🔴 Matches `SPOT_SHADOW_NEAR_Z` and the cube pass by value; depth is
/// `near / distance`, so disagreeing is wrong everywhere.
const PAGE_NEAR: f32 = 0.05;

/// A lamp offset in one face's space, `z` positive in front. 🔴 Unconditional: a point behind the
/// face returns negative `z` for `w` to clip — rejecting per point split seam triangles into false
/// bars of occlusion.
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

/// A lamp offset in one cell's clip space with `xy` multiplied by `w` and `z` = `w`, so the
/// rasterizer divides per fragment.
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

/// Whether a sphere reaches a lamp page's cell: a frustum, so the test is the cell's circumscribing
/// cone, not a box. Conservative — over-emitting costs a discarded triangle, under-emitting a
/// shadow.
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
    // The sphere subtends `asin(radius / distance)`; compared as cosines to spend one `acos`.
    let cos_to = dot(to_centre / distance, axis);
    let angle = acos(clamp(cos_to, -1.0, 1.0));
    let half = acos(clamp(cos_half, -1.0, 1.0));
    return angle <= half + asin(clamp(radius / distance, 0.0, 1.0));
}

/// Cell axis in `xyz`, corner half-angle cosine in `w` — one call, so a caller cannot disagree with
/// itself.
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

/// Metres per texel: the sun's level extent over its texels, or a lamp face's `2 * range` over the
/// level's — the identity `page_level` inverts.
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

/// Bucket = octave of texel size, so lamps and sun wanting the same fineness share survivors with
/// no per-light cull. Sun level `L` is bucket `L`; ⚠️ clamped, finer lamps get finer geometry.
fn page_octave(texel: f32, base: f32, virtual_texels: u32, levels: u32) -> u32 {
    let finest = base / f32(max(virtual_texels, 1u));
    // 🔴 The nudge keeps the anchor: `base` 1.28 is not a power of two, so an ulp low makes `floor`
    // pick the coarser octave. 1e-4 cannot move a real decision.
    let octave = floor(log2(max(texel, 1e-9) / max(finest, 1e-9)) + 1e-4);
    return u32(clamp(octave, 0.0, f32(max(levels, 1u) - 1u)));
}

