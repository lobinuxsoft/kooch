// meshlet_cull.wgsl — frustum culling per meshlet.
//
// One thread per meshlet. Each thread reads its descriptor, tests the
// bounding sphere against the camera's six frustum planes, and either
// appends its meshlet id to the visible list or skips it.
//
// Output:
//   - visible_meshlets: dense array of u32 ids that survived culling
//   - visible_count: atomic counter, also drives indirect draw args
//
// Backface cone culling + Hi-Z occlusion arrive in subsequent PRs of
// epic #117. PR-3 covers frustum-only — already 50-90% of meshlets in
// a typical scene are off-screen.

struct CullParams {
    // Six frustum planes packed as vec4(normal, distance).
    // Plane equation: dot(normal, p) + distance >= 0 = inside.
    planes: array<vec4<f32>, 6>,
    meshlet_count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
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

@compute @workgroup_size(64, 1, 1)
fn cs_cull(@builtin(global_invocation_id) gid: vec3<u32>) {
    let meshlet_id = gid.x;
    if (meshlet_id >= params.meshlet_count) {
        return;
    }

    let desc = descriptors[meshlet_id];
    let center = desc.bounds_center;
    let radius = desc.bounding_radius;

    // Frustum cull: reject if the bounding sphere is fully outside ANY plane.
    var visible = true;
    for (var i = 0u; i < 6u; i = i + 1u) {
        let plane = params.planes[i];
        let signed_dist = dot(plane.xyz, center) + plane.w;
        // Sphere fully outside if its furthest point is still beyond plane.
        if (signed_dist < -radius) {
            visible = false;
            break;
        }
    }

    if (visible) {
        let slot = atomicAdd(&visible_count, 1u);
        visible_meshlets[slot] = meshlet_id;
    }
}
