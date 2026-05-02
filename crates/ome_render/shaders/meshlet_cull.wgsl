// meshlet_cull.wgsl — frustum + backface cone culling per meshlet.
//
// One thread per meshlet. Each thread reads its descriptor and runs:
//   1. frustum: bounding sphere vs the camera's six planes.
//   2. backface cone: dot(normalize(camera - cone_apex), cone_axis) >= cone_cutoff
//      → the camera lies in the meshlet's back-facing half-space; cull.
//
// Surviving meshlets are appended to `visible_meshlets[]` via an
// atomic counter, which the host then mirrors into the indirect-draw
// `instance_count` slot.
//
// Hi-Z occlusion arrives in PR-5 — a separate pass + a depth-pyramid
// binding that this shader will read after the cull dispatcher gains
// a Hi-Z input.

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

@compute @workgroup_size(64, 1, 1)
fn cs_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let meshlet_id = gid.x;
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
