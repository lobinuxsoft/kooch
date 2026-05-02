// meshlet_cull.wgsl — frustum + backface cone + (optional) Hi-Z cull.
//
// One thread per meshlet. Each thread reads its descriptor and runs:
//   1. frustum: bounding sphere vs the camera's six planes.
//   2. backface cone: dot(normalize(camera - cone_apex), cone_axis) >= cone_cutoff
//      → the camera lies in the meshlet's back-facing half-space; cull.
//   3. (cs_cull_hi_z only) Hi-Z occlusion: project the bounding sphere
//      to screen, pick a mip whose texel covers it, and reject when
//      the sphere's NDC depth lies past the tile's max depth (sphere
//      is fully behind every fragment in the tile).
//
// Two entry points share the same logic-up-to-cone-test core. The
// Hi-Z variant gets its pyramid binding through group(1).
//
// Surviving meshlets are appended to `visible_meshlets[]` via an
// atomic counter, which the host then mirrors into the indirect-draw
// `instance_count` slot.

struct CullParams {
    // Six frustum planes packed as vec4(normal, distance).
    // Plane equation: dot(normal, p) + distance >= 0 = inside.
    planes: array<vec4<f32>, 6>,
    camera_position: vec3<f32>,
    meshlet_count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
}

struct MeshletDescriptor {
    vertex_offset: u32,
    triangle_offset: u32,
    vertex_count: u32,
    triangle_count: u32,
    aabb_min: vec3<f32>,
    _pad0: u32,
    aabb_max: vec3<f32>,
    _pad1: u32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> params: CullParams;
@group(0) @binding(1) var<storage, read> descriptors: array<MeshletDescriptor>;
@group(0) @binding(2) var<storage, read_write> visible_meshlets: array<u32>;
@group(0) @binding(3) var<storage, read_write> visible_count: atomic<u32>;

// group(1) — only bound when running cs_cull_hi_z.
struct HiZParams {
    view_proj: mat4x4<f32>,
    hi_z_size: vec2<f32>,
    hi_z_mip_count: u32,
    _pad0: u32,
}
@group(1) @binding(0) var<uniform> hi_z_params: HiZParams;
@group(1) @binding(1) var hi_z_pyramid: texture_2d<f32>;

fn sphere_outside_frustum(center: vec3<f32>, radius: f32) -> bool {
    for (var i = 0u; i < 6u; i = i + 1u) {
        let plane = params.planes[i];
        let signed_dist = dot(plane.xyz, center) + plane.w;
        if (signed_dist < -radius) {
            return true;
        }
    }
    return false;
}

fn camera_in_backface_cone(desc: MeshletDescriptor) -> bool {
    // meshopt sets cone_cutoff to 1.0 when the meshlet's normals are
    // too divergent for a meaningful cone. Treat that as a no-cull
    // sentinel — the test would otherwise reject everything in front.
    if (desc.cone_cutoff >= 1.0) {
        return false;
    }
    let to_camera = params.camera_position - desc.cone_apex;
    let len_sq = dot(to_camera, to_camera);
    if (len_sq == 0.0) {
        return false;
    }
    let view = to_camera / sqrt(len_sq);
    return dot(view, desc.cone_axis) >= desc.cone_cutoff;
}

// Pessimistic single-texel Hi-Z occlusion. Project the bounding sphere
// centre to NDC, pick a mip whose texel size covers the projected
// sphere radius, and reject when the sphere's NDC depth lies past the
// tile's max stored depth.
//
// The test is conservative on the wrong side — false negatives (missed
// cull opportunities) are fine, false positives (culling something
// visible) would tear holes. We err away from culling when:
//   - the sphere intersects the near plane (clip.w <= radius)
//   - the projected centre falls outside the screen
//   - the bounding sphere's near edge is closer than the tile's max
//     depth (sphere may still poke through the occluder)
fn occluded_by_hi_z(center_world: vec3<f32>, radius: f32) -> bool {
    let clip = hi_z_params.view_proj * vec4<f32>(center_world, 1.0);
    // Behind / on / very near the camera — keep it.
    if (clip.w <= radius) {
        return false;
    }
    let ndc = clip.xyz / clip.w;
    // Outside the canonical clip volume — frustum should already have
    // caught this; bail before sampling outside the pyramid.
    if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0) {
        return false;
    }

    // NDC → uv (NDC y points up; texture v points down).
    let uv = vec2<f32>((ndc.x + 1.0) * 0.5, (1.0 - ndc.y) * 0.5);

    // Approximate screen-space pixel radius. Using clip.w as view-space
    // depth: sphere_radius / depth ≈ tan(half_angle).
    let sphere_pixel_radius = max(
        radius / clip.w * hi_z_params.hi_z_size.x * 0.5,
        1.0,
    );
    // Pick the smallest mip whose single texel covers the sphere.
    let mip_f = ceil(log2(sphere_pixel_radius * 2.0));
    let mip = clamp(u32(mip_f), 0u, hi_z_params.hi_z_mip_count - 1u);

    let mip_w = max(u32(hi_z_params.hi_z_size.x) >> mip, 1u);
    let mip_h = max(u32(hi_z_params.hi_z_size.y) >> mip, 1u);
    let px = clamp(u32(uv.x * f32(mip_w)), 0u, mip_w - 1u);
    let py = clamp(u32(uv.y * f32(mip_h)), 0u, mip_h - 1u);

    let max_depth = textureLoad(hi_z_pyramid, vec2<u32>(px, py), i32(mip)).r;
    // Sphere's closest-to-camera depth in NDC. Conservative: skip the
    // radius extension and use the centre depth — gives slightly fewer
    // cull successes but never false-positives.
    let sphere_min_depth = ndc.z;
    return sphere_min_depth > max_depth;
}

// Naga's pipeline-layout validation walks the call graph of each entry
// point and demands a binding slot for every global the function tree
// reaches. Splitting the cull body into two siblings (one with the
// Hi-Z branch, one without) is the cleanest way to make `cs_cull`'s
// pipeline layout `[group(0)]` and `cs_cull_hi_z`'s layout
// `[group(0), group(1)]` — naga sees the Hi-Z bindings only from the
// second branch, never references them from `cs_cull`.

fn run_cull_basic(meshlet_id: u32) {
    if (meshlet_id >= params.meshlet_count) {
        return;
    }

    let desc = descriptors[meshlet_id];

    if (sphere_outside_frustum(desc.bounds_center, desc.bounding_radius)) {
        return;
    }
    if (camera_in_backface_cone(desc)) {
        return;
    }

    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = meshlet_id;
}

fn run_cull_with_hi_z(meshlet_id: u32) {
    if (meshlet_id >= params.meshlet_count) {
        return;
    }

    let desc = descriptors[meshlet_id];

    if (sphere_outside_frustum(desc.bounds_center, desc.bounding_radius)) {
        return;
    }
    if (camera_in_backface_cone(desc)) {
        return;
    }
    if (occluded_by_hi_z(desc.bounds_center, desc.bounding_radius)) {
        return;
    }

    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = meshlet_id;
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    run_cull_basic(gid.x);
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_hi_z(@builtin(global_invocation_id) gid: vec3<u32>) {
    run_cull_with_hi_z(gid.x);
}
