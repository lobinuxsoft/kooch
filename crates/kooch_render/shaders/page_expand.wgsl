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
@group(0) @binding(1) var<storage, read> page_list: array<vec2<u32>>;
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
        let slot = atomicAdd(&page_counts[buckets + 2u], 1u);
        if slot >= raster.chain.y {
            atomicAdd(&page_counts[buckets + 3u], 1u);
            return;
        }
        pairs[slot] = vec4<u32>(entry.x, entry.y, packed, 0u);
        return;
    }

    let basis = sun_basis(raster.sun.xyz);
    let centre = sun_centre(raster.eye.xyz, basis, raster.world.x, raster.space.z, id.level);
    let rect = sun_page_rect(id.level, id.cell, raster.world.x, raster.space.z, centre);

    // Sphere against the page's box, in the sun's own frame. The depth
    // axis is the orthographic span rather than the page's width: a
    // caster far above the page still writes into it, which is the whole
    // point of a shadow.
    // Page index zero, which is one thread per survivor and exactly the
    // fan-out a scatter would run at. Placed BEFORE the rejections
    // below: the cost being counted is what the other shape would pay
    // whether or not this pair survives.
    if gid.x < meshlets {
        count_scatter(level, buckets, bounds, radius);
    }

    let plane = sun_plane(bounds, basis);
    let along = dot(bounds - raster.eye.xyz, basis[2]);
    let half = rect.z * 0.5 + radius;
    if abs(plane.x - rect.x) > half || abs(plane.y - rect.y) > half {
        return;
    }
    if abs(along) > raster.world.y + radius {
        return;
    }

    let slot = atomicAdd(&page_counts[buckets + 2u], 1u);
    if slot >= raster.chain.y {
        atomicAdd(&page_counts[buckets + 3u], 1u);
        return;
    }
    pairs[slot] = vec4<u32>(entry.x, entry.y, packed, 0u);
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
