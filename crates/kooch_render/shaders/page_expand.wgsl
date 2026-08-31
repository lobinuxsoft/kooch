// page_expand.wgsl — which meshlet has to be drawn into which page
// (#866).
//
// CONCATENATED after `page_table.wgsl`. One dispatch per clipmap level,
// sized indirectly by `cs_expand_args`, because the two numbers it
// multiplies — resident pages and surviving meshlets — only exist on the
// GPU.
//
// # Why the pair list is the whole trick
//
// A shadow page is a 128-texel view of the world and a scene has
// thousands of meshlets. Rasterising every meshlet into every page is
// the cost virtual shadow maps exist to avoid; rasterising a meshlet
// once, into the pages it actually touches, is what makes 1681 pages
// affordable. This pass is where "actually touches" is decided, and it
// is one sphere against one box.
//
// The pair carries the cull's own packed `(instance << 16 | meshlet)`,
// so it is self-describing: the draw never learns which level produced
// it, which is what lets every level share ONE `draw_indirect`.

struct MeshletDescriptor {
    vertex_offset: u32,
    triangle_offset: u32,
    vertex_count: u32,
    triangle_count: u32,
    aabb_min: vec3<f32>,
    parent_meshlet_index: u32,
    aabb_max: vec3<f32>,
    lod_error: f32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    group_index: u32,
    children_group_index: u32,
    lod_level: u32,
    _pad4: u32,
    _pad5: u32,
}

// Stride 96 B, mirroring the cull side. A mismatch here reads a
// transform from the middle of the previous instance.
struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    flags: u32,
    _pad1: u32,
    _pad2: u32,
}

/// Which level this dispatch is expanding. A dynamic uniform offset,
/// the way the cascade matrix already is.
///
/// 🔴 THREE separate `u32` and not a `vec3<u32>`. A `vec3<u32>` aligns
/// to 16, so the padding would start at offset 16 and the struct would
/// measure **32** bytes against the Rust mirror's 16 — which is exactly
/// what it did. It compiles, it validates, and it fails at bind time
/// with *"bound with size 16 where the shader expects 32"*, once per
/// frame forever.
struct ExpandLevel {
    level: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> raster: PageRaster;
@group(0) @binding(1) var<storage, read> page_list: array<vec4<u32>>;
@group(0) @binding(2) var<storage, read_write> page_counts: array<atomic<u32>>;
// x the index into `page_list`, y the cull's packed `(instance, meshlet)`.
// Four words: the page, its slot, the cull's packed pair, a spare.
@group(0) @binding(3) var<storage, read_write> pairs: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read> visible_counts: array<u32>;
@group(0) @binding(5) var<uniform> expand: ExpandLevel;
// 🔴 In group 0 and not a group of its own: `max_bind_groups` is FOUR
// and the meshlet pool, the survivor list and the instances already own
// the other three. This is also the eighth storage buffer the stage
// binds, which is the whole downlevel budget — the next reader of this
// pass has to displace something.
@group(0) @binding(6) var<storage, read> lights: array<ClusterLight>;
// The page pyramid, and a TEXTURE rather than the ninth storage buffer
// the line above says does not exist. `page_pyramid.wgsl` builds it and
// `page_overlap.wgsl` reads it; mip 0 holds `listing + 1`, so a descent
// that reaches a texel has the pair without ever binding the table.
@group(0) @binding(7) var page_pyramid: texture_2d_array<u32>;

@group(1) @binding(0) var<storage, read> descriptors: array<MeshletDescriptor>;
@group(2) @binding(0) var<storage, read> visible_meshlets: array<u32>;
@group(3) @binding(0) var<storage, read> instances: array<MeshInstance>;

const EXPAND_GROUP: u32 = 64u;

/// The largest axis scale a transform applies, which is what a bounding
/// radius has to be multiplied by. Mirrors `instance_world_scale` in the
/// cull.
fn transform_scale(m: mat4x4<f32>) -> f32 {
    return max(length(m[0].xyz), max(length(m[1].xyz), length(m[2].xyz)));
}

/// Nodes one descent may hold.
///
/// A 4-ary depth-first walk that pushes four children and pops one
/// needs `3 * depth + 1`, and the four seeds at the entry mip add three
/// more. Eight mips is a 128-page level, the widest side the clipmap
/// builds, so 28 is the true bound; 32 is that with room. The guard at
/// the push is for the day the page size changes, not for today —
/// overflowing would DROP a caster, so it is counted rather than
/// silent.
const PAGE_STACK: u32 = 32u;

fn pack_node(x: u32, y: u32, mip: u32) -> u32 {
    return (mip << 24u) | (x << 12u) | y;
}

/// One page of the sun's clipmap against one meshlet: the tests, then
/// the pair.
///
/// 🔴 Shared by both shapes on purpose. The paired pass reaches a page
/// from the compacted list and the inverted one reaches it from the
/// geometry, but what makes a pair SURVIVE has to be the same text in
/// both — a second copy free to drift is a picture that changes with a
/// setting that was only supposed to change the cost.
fn sun_pair(
    entry: vec4<u32>,
    level: u32,
    cell: vec2<u32>,
    buckets: u32,
    packed: u32,
    bounds: vec3<f32>,
    radius: f32,
    basis: mat3x3<f32>,
) {
    // Sphere against the page's box, in the sun's own frame. The depth
    // axis is the orthographic span rather than the page's width: a
    // caster far above the page still writes into it, which is the
    // whole point of a shadow.
    let rect = sun_page_rect(level, cell, raster.eye.xyz, basis, raster.world.x, raster.space.z);
    let plane = sun_plane(bounds, basis);
    let along = dot(bounds - raster.eye.xyz, basis[2])
        + sun_drift(raster.eye.xyz, basis, raster.world.x, raster.space.z, level);
    let half = rect.z * 0.5 + radius;
    if abs(plane.x - rect.x) > half || abs(plane.y - rect.y) > half {
        return;
    }
    if abs(along) > raster.world.y + radius {
        return;
    }
    // #949 — Olsson §4's receiver bound on the sun's axis. The lamp's
    // twin sits in `cs_expand`, and the asymmetry is the whole point: a
    // lamp is a point, so a radius bounds it in every direction at
    // once. The sun is directional, so only the FAR side can go. A
    // caster nearer the sun than every receiver here still shadows
    // them, however far away it is, and that side is never rejected.
    //
    // `entry.z` holds `along + span` max-reduced over this page's
    // receivers — the same bias, from the same snapped origin, that
    // `mark_sun` wrote. Zero means the marking recorded nothing, which
    // keeps the caster: the safe way for this to be wrong.
    if raster.eye.w != 0.0
        && entry.z != 0u
        && (along + raster.world.y) - radius > bitcast<f32>(entry.z)
    {
        atomicAdd(&page_counts[buckets * 3u + 6u], 1u);
        return;
    }

    let slot = atomicAdd(&page_counts[buckets + 2u], 1u);
    if slot >= raster.chain.y {
        atomicAdd(&page_counts[buckets + 3u], 1u);
        return;
    }
    pairs[slot] = vec4<u32>(entry.x, entry.y, packed, 0u);
}

/// Walks the pyramid down to the listed pages under `rect`, and pairs
/// the meshlet with each.
///
/// # 🔴 Why a descent and not a walk
///
/// The rectangle a meshlet covers is up to 16384 cells at the finest
/// levels while twenty pages are listed there — walking it is the
/// scatter shape `count_scatter` measures and the reason it lost. The
/// descent visits a node only when the node says something below it is
/// being drawn, so it costs the LISTED pages under the rectangle plus
/// the depth of the chain, not the rectangle's area.
///
/// It starts at `overlap_mip`'s level rather than at the root: the four
/// texels that already answer the rectangle are the four subtrees worth
/// entering, so a small rectangle never pays for the levels above it.
fn expand_descend(
    rect: vec4<u32>,
    level: u32,
    mips: u32,
    buckets: u32,
    packed: u32,
    bounds: vec3<f32>,
    radius: f32,
    basis: mat3x3<f32>,
) {
    var stack: array<u32, PAGE_STACK>;
    var top = 0u;
    let mip = overlap_mip(rect, mips);
    let a = vec2<u32>(rect.x >> mip, rect.y >> mip);
    let b = vec2<u32>(rect.z >> mip, rect.w >> mip);
    // Up to four seeds, deduplicated: a rectangle that collapses to one
    // texel per axis must not enter the same subtree twice, or every
    // page under it is paired four times.
    for (var seed = 0u; seed < 4u; seed = seed + 1u) {
        let wide = (seed & 1u) != 0u;
        let tall = (seed & 2u) != 0u;
        if (wide && a.x == b.x) || (tall && a.y == b.y) {
            continue;
        }
        stack[top] = pack_node(select(a.x, b.x, wide), select(a.y, b.y, tall), mip);
        top = top + 1u;
    }

    loop {
        if top == 0u {
            break;
        }
        top = top - 1u;
        let node = stack[top];
        let here = node >> 24u;
        let x = (node >> 12u) & 0xfffu;
        let y = node & 0xfffu;
        let value = textureLoad(
            page_pyramid,
            vec2<i32>(i32(x), i32(y)),
            i32(level),
            i32(here),
        ).x;
        if value == 0u {
            continue;
        }
        if here == 0u {
            // The one place this shape pays what the other one pays:
            // one page tested against one meshlet. Counted here rather
            // than derived from `pages * meshlets`, which is the
            // paired shape's product and says nothing about this one.
            atomicAdd(&page_counts[buckets * 3u + 7u], 1u);
            sun_pair(page_list[value - 1u], level, vec2<u32>(x, y), buckets, packed, bounds, radius, basis);
            continue;
        }
        let below = here - 1u;
        for (var child = 0u; child < 4u; child = child + 1u) {
            let cx = x * 2u + (child & 1u);
            let cy = y * 2u + (child >> 1u);
            let low = vec2<u32>(cx << below, cy << below);
            let high = vec2<u32>(((cx + 1u) << below) - 1u, ((cy + 1u) << below) - 1u);
            if high.x < rect.x || low.x > rect.z || high.y < rect.y || low.y > rect.w {
                continue;
            }
            if top >= PAGE_STACK {
                atomicAdd(&page_counts[buckets * 3u + 8u], 1u);
                continue;
            }
            stack[top] = pack_node(cx, cy, below);
            top = top + 1u;
        }
    }
}

/// The inverted expansion: from ONE meshlet to the pages it lands in.
///
/// # 🔴 The direction is the point
///
/// The paired shape decides its two halves apart — the marking makes a
/// page resident because a RECEIVER asked for it, the cull produces
/// survivors from the light's own view — and nothing checks that they
/// agree. Unreal walk instances and ask whether the pages an instance
/// covers are resident, which is one decision and therefore cannot
/// disagree with itself. This is that arrangement.
///
/// ⚠️ The page window is TOROIDAL, so a rectangle in absolute page
/// indices can wrap the grid's seam and become up to four rectangles in
/// the table's own coordinates. Descending the unwrapped rectangle
/// would read blocks that hold the far side of the world.
fn expand_geometry(
    level: u32,
    buckets: u32,
    packed: u32,
    bounds: vec3<f32>,
    radius: f32,
) {
    let side = raster.space.z;
    let s = f32(side);
    let basis = sun_basis(raster.sun.xyz);
    let width = raster.world.x * exp2(f32(level)) / s;
    let plane = sun_plane(bounds, basis);
    let low = sun_window(raster.eye.xyz, basis, raster.world.x, side, level);
    let high = low + vec2<f32>(s - 1.0);
    // The same mapping `sun_cell` marks with, widened by the sphere.
    let first = floor((plane - vec2<f32>(radius)) / width);
    let last = floor((plane + vec2<f32>(radius)) / width);
    if last.x < low.x || last.y < low.y || first.x > high.x || first.y > high.y {
        return;
    }
    let lo = max(first, low);
    let hi = min(last, high);
    let start = vec2<u32>(wrap_to(lo, s));
    let span = vec2<u32>(hi - lo);

    var xs: array<vec2<u32>, 2>;
    var ys: array<vec2<u32>, 2>;
    var nx = 1u;
    var ny = 1u;
    xs[0] = vec2<u32>(start.x, min(start.x + span.x, side - 1u));
    if start.x + span.x >= side {
        xs[1] = vec2<u32>(0u, start.x + span.x - side);
        nx = 2u;
    }
    ys[0] = vec2<u32>(start.y, min(start.y + span.y, side - 1u));
    if start.y + span.y >= side {
        ys[1] = vec2<u32>(0u, start.y + span.y - side);
        ny = 2u;
    }

    let mips = firstLeadingBit(side) + 1u;
    for (var i = 0u; i < nx; i = i + 1u) {
        for (var j = 0u; j < ny; j = j + 1u) {
            expand_descend(
                vec4<u32>(xs[i].x, ys[j].x, xs[i].y, ys[j].y),
                level,
                mips,
                buckets,
                packed,
                bounds,
                radius,
                basis,
            );
        }
    }
}

@compute @workgroup_size(EXPAND_GROUP, 1, 1)
fn cs_expand(@builtin(global_invocation_id) gid: vec3<u32>) {
    let level = expand.level;
    // Sun buckets plus one per lamp; the counters live after all of
    // them. `level` is a BUCKET index — for a lamp it is
    // `chain.x + slot`, bound to the SHARED survivor arena.
    let buckets = raster.chain.x + LAMP_CULLS;
    let pages = min(atomicLoad(&page_counts[level]), raster.chain.z);
    var meshlets = visible_counts[level];
    // A lamp's survivors live in its fixed slice of the arena. The
    // count is written uncapped — that is how an overflowing lamp is
    // visible — so the reader clamps to the slice.
    var survivor_base = 0u;
    if level >= raster.chain.x {
        meshlets = min(meshlets, LAMP_SURVIVORS);
        survivor_base = (level - raster.chain.x) * LAMP_SURVIVORS;
    }
    if pages == 0u || meshlets == 0u {
        return;
    }
    // 🔴 The sun's buckets only, and the restriction is the pyramid's:
    // it covers ONE clipmap. A lamp's page is a frustum from a point on
    // one of six faces, which is a different grid and a different
    // rectangle — see the two shapes below.
    let inverted = raster.layer.w != 0u && level < raster.chain.x;
    if inverted {
        // One thread per SURVIVOR, not per pair. `cs_expand_args` sizes
        // the dispatch the same way, and the two have to agree: a
        // dispatch still sized `pages * meshlets` would run the descent
        // once per page and pair everything `pages` times over.
        if gid.x >= meshlets {
            return;
        }
        let packed = visible_meshlets[survivor_base + gid.x];
        let inst = instances[packed >> 16u];
        let desc = descriptors[packed & 0xffffu];
        let bounds = (inst.transform * vec4<f32>(desc.bounds_center, 1.0)).xyz;
        let radius = desc.bounding_radius * transform_scale(inst.transform);
        count_scatter(level, buckets, bounds, radius);
        expand_geometry(level, buckets, packed, bounds, radius);
        return;
    }
    if gid.x >= pages * meshlets {
        return;
    }
    // Page-major, so the threads that share a page share its rect and
    // the divergent half is the meshlet fetch.
    let entry = page_list[level * raster.chain.z + gid.x / meshlets];
    let packed = visible_meshlets[survivor_base + gid.x % meshlets];

    let inst = instances[packed >> 16u];
    let desc = descriptors[packed & 0xffffu];
    let bounds = (inst.transform * vec4<f32>(desc.bounds_center, 1.0)).xyz;
    let radius = desc.bounding_radius * transform_scale(inst.transform);

    let id = page_decode(
        entry.x,
        raster.views.y,
        raster.space.x,
        raster.space.y,
        raster.space.z,
        raster.space.w,
        raster.pool.w,
    );

    // 🔴 A lamp's page is a FRUSTUM from a point and the sun's is a
    // slab. The same sphere test against a box is wrong at every
    // distance except the one the box was built at, which is why the
    // two branches here are two shapes rather than two constants.
    if !id.is_sun {
        if id.light >= arrayLength(&lights) {
            return;
        }
        let light = lights[id.light];
        var to_centre = bounds - light.position;
        // The cell's cone is built in the face's own frame, so a spot's
        // candidates rotate into it the way every other pass does — see
        // `spot_local`.
        if light.kind == PAGE_KIND_SPOT {
            to_centre = spot_local(light.direction, to_centre);
        }
        let cone = cell_cone(id.face, id.cell, level_side_of(id.level, raster.space.z));
        if !cell_reaches(cone.xyz, cone.w, to_centre, radius, light.range) {
            return;
        }
        // #940 — Olsson §4's receiver bound, at page granularity: a
        // caster whose NEAREST point lies beyond this page's furthest
        // receiver occludes nothing the frame shades. `entry.z` is the
        // marking's radial atomicMax; zero means no receiver was
        // recorded and nothing is rejected. The spot rotation above
        // preserves length, so one comparison serves both kinds.
        if raster.eye.w != 0.0
            && entry.z != 0u
            && length(to_centre) - radius > bitcast<f32>(entry.z)
        {
            atomicAdd(&page_counts[buckets * 3u + 5u], 1u);
            return;
        }
        let slot = atomicAdd(&page_counts[buckets + 2u], 1u);
        if slot >= raster.chain.y {
            atomicAdd(&page_counts[buckets + 3u], 1u);
            return;
        }
        pairs[slot] = vec4<u32>(entry.x, entry.y, packed, 0u);
        return;
    }

    // Page index zero, which is one thread per survivor and exactly the
    // fan-out a scatter would run at. Placed BEFORE the pair's own
    // rejections: the cost being counted is what the other shape would
    // pay whether or not this pair survives.
    if gid.x < meshlets {
        count_scatter(level, buckets, bounds, radius);
    }
    sun_pair(
        entry,
        id.level,
        id.cell,
        buckets,
        packed,
        bounds,
        radius,
        sun_basis(raster.sun.xyz),
    );
}

/// What the OTHER shape of this pass would have cost, counted without
/// running it.
///
/// # The two shapes
///
/// A meshlet has to reach the pages it overlaps, and there are exactly
/// two ways to find them. The pass above PAIRS: every resident page
/// against every survivor, one sphere-box test each, `pages ×
/// meshlets` of them. The alternative SCATTERS: one thread per
/// survivor walks the cells its own bounding sphere covers and looks
/// each one up in the table, `sum over meshlets of cells` of them.
///
/// Neither wins everywhere, and that is the point. A page at level 0 is
/// `base / side` wide — centimetres — so a metre-wide meshlet covers
/// thousands of cells while only a handful of pages are resident:
/// pairing wins by orders of magnitude. At level 12 a page is hundreds
/// of metres wide, every meshlet lands in one cell, and pairing spends
/// the whole level proving misses against pages the meshlet was never
/// near.
///
/// 🔴 This counts the scatter's cells and does NOT scatter. The
/// previous attempt at this shipped the scatter for every level at once
/// on an unmeasured guess and cost two thirds of the frame rate. The
/// number below is what decides the shape per level — and it is
/// measured before anything is chosen, not after.
///
/// Free to run: it rides the threads that already exist for page index
/// zero, so it adds arithmetic to `meshlets` threads and no dispatch.
fn count_scatter(level: u32, buckets: u32, bounds: vec3<f32>, radius: f32) {
    let side = raster.space.z;
    let basis = sun_basis(raster.sun.xyz);
    let centre = sun_centre(raster.eye.xyz, basis, raster.world.x, side, level);
    let extent = raster.world.x * exp2(f32(level));
    let plane = sun_plane(bounds, basis);

    // The same mapping `sun_page_for` marks with, widened by the
    // sphere: the cells a scatter would have to visit.
    let lo = (plane - vec2<f32>(radius) - centre) / extent + vec2<f32>(0.5);
    let hi = (plane + vec2<f32>(radius) - centre) / extent + vec2<f32>(0.5);
    if hi.x < 0.0 || hi.y < 0.0 || lo.x >= 1.0 || lo.y >= 1.0 {
        return;
    }
    let top = f32(side) - 1.0;
    let first = clamp(floor(lo * f32(side)), vec2<f32>(0.0), vec2<f32>(top));
    let last = clamp(floor(hi * f32(side)), vec2<f32>(0.0), vec2<f32>(top));
    let span = last - first + vec2<f32>(1.0);
    atomicAdd(&page_counts[buckets * 2u + 5u + level], u32(span.x * span.y));
}
