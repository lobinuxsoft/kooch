// page_depth.wgsl — rasterising a meshlet into the page it was paired
// with (#866).
//
// CONCATENATED after `page_table.wgsl`. Vertex routing is
// `shadow_depth.wgsl`'s with one change: the matrix is not a uniform,
// it is BUILT PER INSTANCE out of the page the pair names.
//
// # One render pass for the whole clipmap
//
// The atlas is a single depth attachment and every page is a sub-rect of
// it. `page_clip` places a page's own clip space inside that rect, so
// 1681 pages are one `begin_render_pass` and one `draw_indirect` rather
// than 1681 of each. The hardware depth test does winner-takes-all
// exactly as it does for a cascade.
//
// # 🔴 Why this one HAS a fragment shader when `shadow_depth` does not
//
// A triangle wider than its page keeps rasterising past the rect and
// into the neighbouring page, which belongs to another level — a caster
// would appear in a shadow map it was never meant to be in, at the wrong
// scale, and nothing about the result would say why. A scissor would fix
// it and cannot: scissor is pass state and the page changes per
// instance.
//
// So the fragment shader exists to `discard` outside the rect, and the
// cost is early-Z: a discard makes depth writes late. That is the price
// of one pass instead of one per page, and it is the right side of the
// trade by three orders of magnitude.

struct MeshVertexStored {
    position: array<f32, 3>,
    normal: array<f32, 3>,
    uv: array<f32, 2>,
}

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

@group(0) @binding(0) var<uniform> raster: PageRaster;
// 🔴 FOUR words a pair — the virtual page, its physical slot, the
// cull's packed `(instance, meshlet)`, and a spare. It used to be two,
// with the page and slot fetched through `page_list`, and that
// indirection cost this stage a storage binding it does not have:
// `max_storage_buffers_per_shader_stage` is 8 and the mesh pool alone
// spends five.
//
// It also costs LESS memory, because the capacity came down with it.
@group(0) @binding(2) var<storage, read> pairs: array<vec4<u32>>;
// 🔴 A lamp's page is a frustum from the light's own position, so its
// transform cannot come from a uniform the way the sun's does: apex,
// axis and reach are all properties of the light the page names. In
// group 0 because group 2 is the shared one-buffer instance layout and
// widening it would widen every pass that uses it.
@group(0) @binding(1) var<storage, read> lights: array<ClusterLight>;

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

@group(2) @binding(0) var<storage, read> instances: array<MeshInstance>;

struct PageVertex {
    @builtin(position) clip: vec4<f32>,
    // The page's rect in atlas texels, flat: every fragment of the
    // triangle belongs to the same page, and interpolating it would
    // make the clip test disagree with itself across the primitive.
    @location(0) @interpolate(flat) rect: vec4<f32>,
}

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

@vertex
fn vs_page(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> PageVertex {
    var out: PageVertex;
    let pair = pairs[instance_index];
    let inst_id = pair.z >> 16u;
    let meshlet_id = pair.z & 0xffffu;
    let desc = descriptors[meshlet_id];

    let id = page_decode(
        pair.x,
        raster.views.y,
        raster.space.x,
        raster.space.y,
        raster.space.z,
        raster.space.w,
    );
    out.rect = page_atlas_rect(pair.y, raster.views.z, raster.pool.z, raster.pool.w);

    let triangle_idx = vertex_index / 3u;
    let corner_idx = vertex_index % 3u;
    // The draw is indirect with a fixed vertex count per meshlet, so the
    // tail of a meshlet with fewer triangles still runs. Sending those
    // vertices outside the clip volume discards the triangle without a
    // branch anywhere else.
    if triangle_idx >= desc.triangle_count {
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }

    let byte_offset = desc.triangle_offset + triangle_idx * 3u + corner_idx;
    let local_vertex_idx = fetch_local_vertex_index(byte_offset);
    let global_vertex_idx = meshlet_vertices[desc.vertex_offset + local_vertex_idx];
    let v = vertices[global_vertex_idx];
    let pos = vec3<f32>(v.position[0], v.position[1], v.position[2]);
    let world = (instances[inst_id].transform * vec4<f32>(pos, 1.0)).xyz;

    var ndc = vec2<f32>(2.0);
    var depth = 0.0;

    if id.is_sun {
        let basis = sun_basis(raster.sun.xyz);
        // 🔴 The plane is ABSOLUTE and the rect carries the snapped
        // centre. Measuring from the camera instead would put the
        // geometry on a grid that slides with it, which is the shadow
        // crawl `sun_centre` exists to remove.
        let centre = sun_centre(raster.eye.xyz, basis, raster.world.x, raster.space.z, id.level);
        let plane = sun_plane(world, basis);
        let along = dot(world - raster.eye.xyz, basis[2]);
        let page = sun_page_rect(id.level, id.cell, raster.world.x, raster.space.z, centre);
        ndc = (plane - page.xy) / (page.z * 0.5);

        // 🔴 Reversed-Z (ADR 0002): 1 is the near plane and 0 is far, so
        // an empty page reads as "nothing between here and the light"
        // rather than as "everything is shadowed". The clear matches.
        let span = raster.world.y;
        depth = 1.0 - (along + span) / (2.0 * span);
    } else {
        // A lamp's page: a perspective frustum from the light, narrowed
        // to the one cell of the one face the page names.
        //
        // Built here rather than uploaded because there is no per-page
        // uniform to upload it into — a draw covers every page of every
        // light at once, and which page an instance belongs to is only
        // known from the pair it read.
        let light = lights[id.light];
        let offset = world - light.position;
        let side = level_side_of(id.level, raster.space.z);
        // 🔴 NOT rejected per vertex. `cell_face` projects whatever it
        // is given and returns a NEGATIVE `w` for a point behind this
        // face — which is what the clipper is for. Rejecting per vertex
        // pushed one corner of a seam-straddling triangle outside the
        // clip volume while the other two projected normally, and the
        // interpolation between them drew a wedge along every seam. On
        // screen: a straight bar of false occlusion across the pool.
        let face = cell_face(id.face, id.cell, side, offset);
        // Reversed-Z along the face's axis. `depth * w` is the constant
        // PAGE_NEAR, so the rasteriser's own divide reconstructs
        // `PAGE_NEAR / z` exactly at every fragment — the identity
        // `GpuPointShadow` documents for the cube path.
        out.clip = page_clip_w(
            face.xy,
            PAGE_NEAR,
            out.rect,
            raster.world.z,
            face.z,
        );
        return out;
    }

    out.clip = page_clip(ndc, depth, out.rect, raster.world.z);
    return out;
}

@fragment
fn fs_page(in: PageVertex) {
    // The page's own texels and nothing else. See the header: this is
    // the scissor the hardware cannot give per instance.
    let at = in.clip.xy;
    if at.x < in.rect.x || at.x >= in.rect.x + in.rect.z {
        discard;
    }
    if at.y < in.rect.y || at.y >= in.rect.y + in.rect.w {
        discard;
    }
}
