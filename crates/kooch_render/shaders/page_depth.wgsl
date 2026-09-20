// page_depth.wgsl — rasterising a meshlet into the page it was paired with (#866).

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
    // Which layers the instance is in (#1218). `MeshInstance` in `scene.rs`.
    layers: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> raster: PageRaster;
// 🔴 FOUR words a pair — the virtual page, its physical slot, the cull's packed `(instance,
// meshlet)`, and a spare.
@group(0) @binding(2) var<storage, read> pairs: array<vec4<u32>>;
// 🔴 A lamp's page is a frustum from the light's own position, so its transform cannot come from a
// uniform the way the sun's does: apex, axis and reach are all properties of the light the page
// names.
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
    // For a transparent caster's coverage (#1224); read only by the alpha variant.
    @location(1) uv: vec2<f32>,
    @location(2) @interpolate(flat) material: u32,
}

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

/// A vertex placed in its page, plus what the CLIPPER needs to cut the
/// triangle to that page — which the fragment path throws away and the
/// clipped path is entirely built on.
struct PageGeom {
    clip: vec4<f32>,
    rect: vec4<f32>,
    /// The page-local position ALREADY multiplied by `w`, exactly as
    /// `page_clip_w` takes it. In-page is `|local| <= w`.
    local: vec2<f32>,
    w: f32,
    /// The tail of a meshlet with fewer triangles than the draw issues
    /// vertices for. Sent outside the volume by both paths.
    dead: bool,
    uv: vec2<f32>,
    material: u32,
}

/// One body, two entry points. Splitting it was the point: the clipped path differs from the
/// fragment path in what it DECLARES, never in where it puts a vertex, and a second copy of this
/// arithmetic is how the two paths would quietly stop agreeing.
fn page_geometry(vertex_index: u32, instance_index: u32) -> PageGeom {
    var out: PageGeom;
    out.local = vec2<f32>(0.0);
    out.w = 1.0;
    out.dead = false;
    out.uv = vec2<f32>(0.0);
    out.material = 0u;
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
        raster.pool.w,
    );
    out.rect = page_atlas_rect(pair.y, raster.views.z, raster.pool.z, raster.pool.w);

    // 🔴 This pass owns ONE layer, and a page in another one would land
    // on the same texels of the wrong layer — corruption, not a miss.
    // Out of the clip volume, the same way the meshlet tail below is.
    if page_layer(pair.y, raster.views.z) != raster.layer.x {
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.dead = true;
        return out;
    }

    let triangle_idx = vertex_index / 3u;
    let corner_idx = vertex_index % 3u;
    // The draw is indirect with a fixed vertex count per meshlet, so the tail of a meshlet with
    // fewer triangles still runs. Sending those vertices outside the clip volume discards the
    // triangle without a branch anywhere else.
    if triangle_idx >= desc.triangle_count {
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.dead = true;
        return out;
    }

    let byte_offset = desc.triangle_offset + triangle_idx * 3u + corner_idx;
    let local_vertex_idx = fetch_local_vertex_index(byte_offset);
    let global_vertex_idx = meshlet_vertices[desc.vertex_offset + local_vertex_idx];
    let v = vertices[global_vertex_idx];
    let pos = vec3<f32>(v.position[0], v.position[1], v.position[2]);
    let world = (instances[inst_id].transform * vec4<f32>(pos, 1.0)).xyz;
    out.uv = vec2<f32>(v.uv[0], v.uv[1]);
    out.material = instances[inst_id].material_id;

    var ndc = vec2<f32>(2.0);
    var depth = 0.0;

    if id.is_sun {
        let basis = sun_basis(raster.sun.xyz);
        // 🔴 The plane is ABSOLUTE and the rect carries the snapped centre. Measuring from the
        // camera instead would put the geometry on a grid that slides with it, which is the shadow
        // crawl `sun_centre` exists to remove.
        let centre = sun_centre(raster.eye.xyz, basis, raster.world.x, raster.space.z, id.level);
        let plane = sun_plane(world, basis);
        // 🔴 And so is the depth origin, for the same reason. Measured
        // from the raw camera it slid every frame, which voided every
        // cached page every frame. See `sun_drift`.
        let along = dot(world - raster.eye.xyz, basis[2])
            + sun_drift(raster.eye.xyz, basis, raster.world.x, raster.space.z, id.level);
        let page = sun_page_rect(id.level, id.cell, raster.eye.xyz, basis, raster.world.x, raster.space.z);
        ndc = (plane - page.xy) / (page.z * 0.5);

        // 🔴 Reversed-Z (ADR 0002): 1 is the near plane and 0 is far, so
        // an empty page reads as "nothing between here and the light"
        // rather than as "everything is shadowed". The clear matches.
        let span = raster.world.y;
        depth = 1.0 - (along + span) / (2.0 * span);
    } else {
        // A lamp's page: a perspective frustum from the light, narrowed to the one cell of the one
        // face the page names.
        let light = lights[id.light];
        var offset = world - light.position;
        // A spot's page projects along the SPOT's axis. See
        // `spot_local`: the writer, this raster and the reader rotate
        // with the same basis or the page holds someone else's depth.
        if light.kind == PAGE_KIND_SPOT {
            offset = spot_local(light.direction, offset);
        }
        let side = level_side_of(id.level, raster.space.z);
        // 🔴 NOT rejected per vertex. `cell_face` projects whatever it is given and returns a
        // NEGATIVE `w` for a point behind this face — which is what the clipper is for.
        let face = cell_face(id.face, id.cell, side, offset);
        // Reversed-Z along the face's axis. `depth * w` is the constant PAGE_NEAR, so the
        // rasteriser's own divide reconstructs `PAGE_NEAR / z` exactly at every fragment — the
        // identity `GpuPointShadow` documents for the cube path.
        out.local = face.xy;
        out.w = face.z;
        out.clip = page_clip_w(
            face.xy,
            PAGE_NEAR,
            out.rect,
            raster.world.z,
            face.z,
        );
        return out;
    }

    // The sun's page is orthographic, so `w` really is 1 and the local
    // position really is the NDC.
    out.local = ndc;
    out.w = 1.0;
    out.clip = page_clip(ndc, depth, out.rect, raster.world.z);
    return out;
}

@vertex
fn vs_page(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> PageVertex {
    let geom = page_geometry(vertex_index, instance_index);
    var out: PageVertex;
    out.clip = geom.clip;
    out.rect = geom.rect;
    out.uv = geom.uv;
    out.material = geom.material;
    return out;
}

// `[0]` the count, then the physical slot of every page the compaction
// listed this dispatch. Only `vs_page_clear` reads it.
@group(0) @binding(3) var<storage, read> clear_dirty: array<u32>;

// One quad per dirty page at far depth — the per-page replacement for the whole-layer clear the
// content cache retired. Reversed-Z: 0 is far, so a wiped page reads as "nothing between here and
// the light".
@vertex
fn vs_page_clear(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> @builtin(position) vec4<f32> {
    let slot = clear_dirty[1u + instance_index];
    // The clear wipes a page's rect, so a slot from another layer would
    // wipe a page that is not dirty. Same gate as the depth draw.
    if page_layer(slot, raster.views.z) != raster.layer.x {
        return vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    let rect = page_atlas_rect(slot, raster.views.z, raster.pool.z, raster.pool.w);
    // Strip order: (-1,-1) (1,-1) (-1,1) (1,1).
    let corner = vec2<f32>(
        f32(vertex_index & 1u),
        f32(vertex_index >> 1u),
    ) * 2.0 - vec2<f32>(1.0);
    return page_clip(corner, 0.0, rect, raster.world.z);
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
