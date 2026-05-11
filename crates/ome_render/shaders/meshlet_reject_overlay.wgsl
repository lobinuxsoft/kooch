// meshlet_reject_overlay.wgsl — reject-reason debug overlay (#454.4).
//
// Reads `reject_reasons[]` (written by cs_cull_scene_pool_atomic
// when CullParams.debug_active != 0), and for every thread whose
// reason matches the host-supplied `selected_reason` projects the
// owning meshlet's world-space AABB to screen and rasterises a
// 1-pixel wireframe rectangle on top of the deferred colour image.
//
// The overlay runs as a compute pass (not a render pass) so it can
// write through the colour texture's existing storage_binding usage
// — adding RENDER_ATTACHMENT to the deferred target just for this
// debug pass is a needless API surface bump. The wireframe style
// keeps the underlying scene visible inside each rejection box,
// which is what makes the visualisation diagnostic in the first
// place: an artist needs to see WHICH cluster's bounds disagree
// with the scene's macro AABB after a mesh edit.
//
// Threading: one thread per cull thread (= instance_count ×
// meshlets_per_mesh). The hot path does an SSBO load + uniform
// compare + early return; threads whose reason matches walk the
// 8-corner projection and write 2·(rect_w + rect_h) pixels along
// the rectangle's perimeter. Per-thread cost is bounded by the
// rectangle's screen footprint; frustum-rejected clusters are
// typically near the viewport edge and project to small rectangles.

struct OverlayParams {
    view_proj: mat4x4<f32>,
    screen_size: vec2<u32>,
    selected_reason: u32,
    line_thickness_px: u32,
}

// Mirror of `MeshletDescriptor` in `meshlet_cull/common.wgsl` —
// kept in lock-step manually because WGSL has no #include and we
// don't pre-process the shader source. If the cull-shader copy
// gains or reorders fields, this struct MUST be updated or the
// AABB read below will pick up garbage.
struct PoolMeshletDescriptor {
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

// Mirror of `PoolMeshDescriptor` in `meshlet_cull/pool.wgsl`.
struct PoolMeshDescriptor {
    first_meshlet: u32,
    meshlet_count: u32,
    vertex_offset: u32,
    meshlet_vertex_offset: u32,
    meshlet_triangle_offset: u32,
    group_base: u32,
    group_count: u32,
    _pad0: u32,
}

// Mirror of `MeshInstance` in `meshlet_cull/scene.wgsl`.
struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// Mirror of `SceneCullParams` in `meshlet_cull/scene.wgsl`. Reused
// here so the overlay can read `instance_count` and `meshlets_per_mesh`
// from the same UBO the cull pass already populates.
struct SceneCullParams {
    instance_count: u32,
    meshlets_per_mesh: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(0) @binding(0) var<uniform> params: OverlayParams;
@group(0) @binding(1) var color_target: texture_storage_2d<rgba8unorm, write>;

@group(1) @binding(0) var<storage, read> mesh_descriptors: array<PoolMeshDescriptor>;
@group(1) @binding(1) var<storage, read> meshlets: array<PoolMeshletDescriptor>;

@group(2) @binding(0) var<storage, read> instances: array<MeshInstance>;
@group(2) @binding(1) var<uniform> scene_params: SceneCullParams;

// Declared `read_write` to match the cull pipeline's debug_bgl
// (the same handle this overlay reuses). The overlay only needs
// LOAD access semantically, but WGSL/wgpu reject pipelines whose
// shader access is a strict subset of the layout's access — the
// two must agree exactly.
@group(3) @binding(0) var<storage, read_write> reject_reasons: array<u32>;

// Reason → flat overlay colour. Mirrors the LUT planned for the
// triangle-density / overdraw heatmaps so the artist can build a
// single colour vocabulary across every advanced debug mode:
//   2 = frustum   → bright yellow
//   3 = backface  → bright blue
//   4 = hi-z      → bright red
//   5 = lod       → cyan (debug-only; #454.5 follow-up surfaces it)
fn reason_color(reason: u32) -> vec4<f32> {
    if (reason == 2u) {
        return vec4<f32>(1.0, 0.95, 0.1, 1.0);
    }
    if (reason == 3u) {
        return vec4<f32>(0.15, 0.45, 1.0, 1.0);
    }
    if (reason == 4u) {
        return vec4<f32>(1.0, 0.2, 0.2, 1.0);
    }
    if (reason == 5u) {
        return vec4<f32>(0.2, 1.0, 0.95, 1.0);
    }
    return vec4<f32>(1.0, 1.0, 1.0, 1.0);
}

// Projects a world-space corner to screen pixel coords. Returns
// `false` when the corner sits at or behind the near plane — the
// caller must drop the entire AABB in that case rather than emit a
// rectangle from a partially flipped projection.
fn project_world(world: vec3<f32>, out: ptr<function, vec2<f32>>) -> bool {
    let clip = params.view_proj * vec4<f32>(world, 1.0);
    if (clip.w <= 0.0001) {
        return false;
    }
    let ndc = clip.xy / clip.w;
    // NDC → pixel coords. NDC y-up flipped to texture v-down.
    let uv = vec2<f32>((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5);
    *out = uv * vec2<f32>(f32(params.screen_size.x), f32(params.screen_size.y));
    return true;
}

// Writes the overlay colour into a horizontal pixel run [x0, x1] at
// row `y`, expanded vertically by `thickness - 1` rows below for
// readability. Bounds-clamped to the screen.
fn paint_h_line(x0: u32, x1: u32, y: u32, thickness: u32, color: vec4<f32>) {
    let max_x = params.screen_size.x;
    let max_y = params.screen_size.y;
    if (y >= max_y) {
        return;
    }
    let lo = min(x0, x1);
    let hi = min(max(x0, x1), max_x - 1u);
    let lo_clamped = min(lo, max_x - 1u);
    for (var x = lo_clamped; x <= hi; x = x + 1u) {
        for (var t = 0u; t < thickness; t = t + 1u) {
            let yy = y + t;
            if (yy >= max_y) {
                break;
            }
            textureStore(color_target, vec2<u32>(x, yy), color);
        }
    }
}

fn paint_v_line(y0: u32, y1: u32, x: u32, thickness: u32, color: vec4<f32>) {
    let max_x = params.screen_size.x;
    let max_y = params.screen_size.y;
    if (x >= max_x) {
        return;
    }
    let lo = min(y0, y1);
    let hi = min(max(y0, y1), max_y - 1u);
    let lo_clamped = min(lo, max_y - 1u);
    for (var y = lo_clamped; y <= hi; y = y + 1u) {
        for (var t = 0u; t < thickness; t = t + 1u) {
            let xx = x + t;
            if (xx >= max_x) {
                break;
            }
            textureStore(color_target, vec2<u32>(xx, y), color);
        }
    }
}

@compute @workgroup_size(64, 1, 1)
fn cs_reject_overlay(@builtin(global_invocation_id) gid: vec3<u32>) {
    let max_meshlets = scene_params.meshlets_per_mesh;
    let total_threads = scene_params.instance_count * max_meshlets;
    if (gid.x >= total_threads) {
        return;
    }
    let reason = reject_reasons[gid.x];
    if (reason != params.selected_reason) {
        return;
    }

    let instance_id = gid.x / max_meshlets;
    let meshlet_offset = gid.x % max_meshlets;
    let inst = instances[instance_id];
    let mesh_desc = mesh_descriptors[inst.mesh_id];
    if (meshlet_offset >= mesh_desc.meshlet_count) {
        return;
    }

    let m = meshlets[mesh_desc.first_meshlet + meshlet_offset];

    // Project all 8 AABB corners through the instance transform +
    // view_proj. Drop the rectangle entirely if any corner falls
    // behind the near plane — partial-clip reconstruction is not
    // worth the shader-side complexity for a debug overlay; the
    // missing visualisation just means the cluster crosses the
    // camera, which is already obvious at a glance.
    var corners: array<vec3<f32>, 8> = array<vec3<f32>, 8>(
        vec3<f32>(m.aabb_min.x, m.aabb_min.y, m.aabb_min.z),
        vec3<f32>(m.aabb_max.x, m.aabb_min.y, m.aabb_min.z),
        vec3<f32>(m.aabb_min.x, m.aabb_max.y, m.aabb_min.z),
        vec3<f32>(m.aabb_max.x, m.aabb_max.y, m.aabb_min.z),
        vec3<f32>(m.aabb_min.x, m.aabb_min.y, m.aabb_max.z),
        vec3<f32>(m.aabb_max.x, m.aabb_min.y, m.aabb_max.z),
        vec3<f32>(m.aabb_min.x, m.aabb_max.y, m.aabb_max.z),
        vec3<f32>(m.aabb_max.x, m.aabb_max.y, m.aabb_max.z),
    );

    var min_px = vec2<f32>(1.0e9, 1.0e9);
    var max_px = vec2<f32>(-1.0e9, -1.0e9);
    for (var i = 0u; i < 8u; i = i + 1u) {
        let world = (inst.transform * vec4<f32>(corners[i], 1.0)).xyz;
        var px: vec2<f32>;
        if (!project_world(world, &px)) {
            return;
        }
        min_px = min(min_px, px);
        max_px = max(max_px, px);
    }

    let max_x = f32(params.screen_size.x);
    let max_y = f32(params.screen_size.y);
    // Drop rectangles that fall entirely outside the viewport. The
    // frustum cull may still mark a cluster rejected when its sphere
    // bound poked into the half-space but its AABB sits fully
    // off-screen; nothing to paint in that case.
    if (max_px.x < 0.0 || max_px.y < 0.0 || min_px.x >= max_x || min_px.y >= max_y) {
        return;
    }

    let lo_x = u32(max(min_px.x, 0.0));
    let lo_y = u32(max(min_px.y, 0.0));
    let hi_x = u32(min(max(max_px.x, 0.0), max_x - 1.0));
    let hi_y = u32(min(max(max_px.y, 0.0), max_y - 1.0));
    if (hi_x <= lo_x || hi_y <= lo_y) {
        return;
    }

    let color = reason_color(reason);
    let thickness = max(params.line_thickness_px, 1u);

    paint_h_line(lo_x, hi_x, lo_y, thickness, color);
    paint_h_line(lo_x, hi_x, hi_y, thickness, color);
    paint_v_line(lo_y, hi_y, lo_x, thickness, color);
    paint_v_line(lo_y, hi_y, hi_x, thickness, color);
}
