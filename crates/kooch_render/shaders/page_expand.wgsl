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
// The page table, read directly. The expansion now SCATTERS — it asks
// which pages a meshlet touches — so it looks pages up by key instead of
// walking a compacted list.
@group(0) @binding(1) var<storage, read> table_keys: array<u32>;
@group(0) @binding(2) var<storage, read_write> page_counts: array<atomic<u32>>;
// x the virtual page, y its physical slot, z the cull's packed
// `(instance, meshlet)`, w unused.
//
// 🔴 Self-describing, where it used to be an index into `page_list`.
// The scatter finds a page by key and already holds everything the draw
// needs, so carrying an index would mean the draw reading a second
// buffer to recover what this pass just had in a register.
@group(0) @binding(3) var<storage, read_write> pairs: array<vec4<u32>>;
@group(0) @binding(4) var<storage, read> visible_counts: array<u32>;
@group(0) @binding(5) var<uniform> expand: ExpandLevel;
// TWO words an entry — slot then age. See `PAGE_CELL`.
@group(0) @binding(6) var<storage, read> table_cells: array<u32>;

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

// Where a virtual page lives, or `PAGE_MISS`. The read half of what the
// marking writes; see `PAGE_DEAD` for why an evicted entry is walked
// past rather than stopped at.
fn expand_lookup(page: u32) -> u32 {
    let entries = raster.pool.x;
    if entries == 0u {
        return PAGE_MISS;
    }
    var probe = page_probe(page, entries);
    for (var i = 0u; i < PAGE_PROBES; i = i + 1u) {
        let key = table_keys[probe];
        if key == PAGE_EMPTY {
            return PAGE_MISS;
        }
        if key == page + 1u {
            return table_cells[probe * PAGE_CELL];
        }
        probe = page_step(probe, entries);
    }
    return PAGE_MISS;
}

// Which pages a meshlet touches, one thread per meshlet.
//
// # 🔴 It SCATTERS. It used to be a cartesian product
//
// The old shape was `pages x meshlets` threads, each asking "does this
// meshlet touch this page". For the sun that is 20 x 1000 and fine. For
// a hundred local lights it is ~1700 pages against thousands of
// meshlets — millions of tests to emit a few thousand pairs, and the
// reason the local raster was written off as "a different machine".
//
// Inverted, a thread projects its own meshlet's bounding sphere into the
// light's plane, walks the CELLS that rect covers, and looks each one up
// in the page table. A meshlet touches one to four pages at a level, so
// the work is `meshlets x 4` rather than `meshlets x pages` — the same
// change Chalmers describes, and what makes local lights affordable at
// all rather than a bigger version of this.
@compute @workgroup_size(EXPAND_GROUP, 1, 1)
fn cs_expand(@builtin(global_invocation_id) gid: vec3<u32>) {
    let level = expand.level;
    let levels = raster.chain.x;
    let meshlets = visible_counts[level];
    if gid.x >= meshlets {
        return;
    }
    let packed = visible_meshlets[gid.x];
    let inst = instances[packed >> 16u];
    let desc = descriptors[packed & 0xffffu];
    let bounds = (inst.transform * vec4<f32>(desc.bounds_center, 1.0)).xyz;
    let radius = desc.bounding_radius * transform_scale(inst.transform);

    let basis = sun_basis(raster.sun.xyz);
    // A caster far above a page still writes into it — that is what a
    // shadow is — so only the plane bounds the search, and the depth
    // axis is the orthographic span.
    let along = dot(bounds - raster.eye.xyz, basis[2]);
    if abs(along) > raster.world.y + radius {
        return;
    }

    let base = raster.world.x;
    let side = raster.space.z;
    let extent = base * exp2(f32(level));
    let width = extent / f32(side);
    let centre = sun_centre(raster.eye.xyz, basis, base, side, level);
    let plane = sun_plane(bounds, basis) - centre;

    // The cells the sphere's rect covers at this level. `sun_page_rect`
    // puts cell (0,0) at `centre - extent/2`, so this inverts it.
    let lo = (plane - vec2<f32>(radius)) / extent + vec2<f32>(0.5);
    let hi = (plane + vec2<f32>(radius)) / extent + vec2<f32>(0.5);
    if hi.x < 0.0 || hi.y < 0.0 || lo.x >= 1.0 || lo.y >= 1.0 {
        return;
    }
    let first = vec2<i32>(floor(clamp(lo, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side)));
    let last = vec2<i32>(floor(clamp(hi, vec2<f32>(0.0), vec2<f32>(0.99999)) * f32(side)));

    let sun_slot = raster.space.w;
    let base_page = raster.views.x * raster.views.y
        + sun_slot * raster.space.x
        + level * side * side;

    for (var y = first.y; y <= last.y; y = y + 1) {
        for (var x = first.x; x <= last.x; x = x + 1) {
            let page = base_page + u32(y) * side + u32(x);
            let slot = expand_lookup(page);
            if slot == PAGE_MISS {
                continue;
            }
            let out = atomicAdd(&page_counts[levels + 2u], 1u);
            if out >= raster.chain.y {
                atomicAdd(&page_counts[levels + 3u], 1u);
                return;
            }
            pairs[out] = vec4<u32>(page, slot, packed, 0u);
        }
    }
    // The rect is a bound, not a hit: a sphere's rect can cover cells the
    // sphere itself misses. `width` is here so the cost of that is
    // visible — a level whose page is far smaller than a caster spends
    // most of these on cells the caster only brushes.
    _ = width;
}
