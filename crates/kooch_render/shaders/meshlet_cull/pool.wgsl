
// ---------------------------------------------------------------
// Phase 1.E.3c — multi-mesh scene-wide cull (#446). One dispatch
// enumerates (instance, meshlet_offset) pairs across the entire
// GlobalMeshPool. Each instance carries `mesh_id` which redirects
// every per-meshlet read through `pool_mesh_descriptors[mesh_id]`.
//
// Why a separate entry instead of extending cs_cull_scene: the
// per-mesh path reads from a single contiguous descriptor array
// bound at group(0)@binding(1); the pool path reads from the
// concatenated pool at group(1) and resolves per-instance mesh
// slices through `pool_mesh_descriptors`. Naga's pipeline-layout
// validation walks the call graph per entry point, so the same
// shader file can host both paths and the unused single-mesh
// `descriptors` binding stays inert when this entry is dispatched.
// ---------------------------------------------------------------

struct PoolMeshDescriptor {
    first_meshlet: u32,
    meshlet_count: u32,
    vertex_offset: u32,
    meshlet_vertex_offset: u32,
    meshlet_triangle_offset: u32,
    // Pool-global base id this mesh's meshlet group_index values were
    // shifted by at registration. Subtract from `m.group_index` to
    // recover the mesh-local group id when computing the per-instance
    // slot in `group_max_err` (#474).
    group_base: u32,
    // Distinct group_ids this mesh contributes (`max_local + 1`); each
    // instance reserves this many slots in `group_max_err` starting at
    // `inst.group_base` (#474). Mirrors Rust `MeshDescriptor.group_count`.
    group_count: u32,
    _pad0: u32,
}

@group(1) @binding(0) var<storage, read> pool_mesh_descriptors: array<PoolMeshDescriptor>;
@group(1) @binding(1) var<storage, read> pool_meshlets: array<MeshletDescriptor>;
// The pool's full bind-group layout exposes vertices /
// meshlet_vertices / meshlet_triangles at bindings 2-4 for the
// rasterizer + deferred shader. The cull pass omits them entirely
// (declaring them here would push the COMPUTE-stage storage-buffer
// count past the wgpu limit of 8), and uses a dedicated cull-only
// pool BGL on the Rust side that matches this two-binding set.


fn lod_pixel_error_pool(lod_error: f32, world_center: vec3<f32>, world_scale: f32) -> f32 {
    // 🔴 Orthographic views do not shrink error with distance.
    //
    // Under perspective the same simplification error covers fewer
    // pixels the further away it is, which is what the divide by `dist`
    // encodes. An orthographic projection magnifies everything equally:
    // the screen error is the world error over the volume's world
    // height, and there is no distance in the relationship at all.
    //
    // Dividing by one anyway makes the test vary across a shadow
    // cascade for no physical reason, so two neighbouring meshlets in
    // the same LOD group fall on opposite sides of the threshold and
    // the surface comes apart. It reads as "some meshlets do not cast a
    // shadow", which is how it was reported. Bevy 0.19 branches on the
    // same condition in `lod_error_is_imperceptible`.
    let world_error = lod_error * world_scale;
    if (params.lod_orthographic == 1u) {
        return world_error * params.lod_error_to_pixel_factor;
    }
    let to_cam = world_center - params.camera_position;
    let dist = max(length(to_cam), 0.0001);
    return world_error * params.lod_error_to_pixel_factor / dist;
}

fn run_cull_scene_pool(thread_id: u32) {
    // `meshlets_per_mesh` carries the worst-case (max) meshlet count
    // across every registered mesh; the per-thread bounds check
    // against the instance's actual mesh_descriptor.meshlet_count
    // covers shorter meshes.
    let max_meshlets = scene_params.meshlets_per_mesh;
    let total_threads = scene_params.instance_count * max_meshlets;
    if (thread_id >= total_threads) {
        return;
    }
    let instance_id = thread_id / max_meshlets;
    let meshlet_offset = thread_id % max_meshlets;

    let inst = instances[instance_id];
    let mesh_desc = pool_mesh_descriptors[inst.mesh_id];

    if (meshlet_offset >= mesh_desc.meshlet_count) {
        return;
    }

    let global_meshlet_idx = mesh_desc.first_meshlet + meshlet_offset;
    let desc = pool_meshlets[global_meshlet_idx];

    // LOD selection — same boundary rule as run_cull_scene; parent
    // indices are pool-global so the lookup stays in pool_meshlets.
    if (desc.parent_meshlet_index != 0xFFFFFFFFu) {
        let parent = pool_meshlets[desc.parent_meshlet_index];
        let world_center_self = (inst.transform * vec4<f32>(desc.bounds_center, 1.0)).xyz;
        let world_center_parent = (inst.transform * vec4<f32>(parent.bounds_center, 1.0)).xyz;
        let scale = instance_world_scale(inst.transform);
        let my_err_px = lod_pixel_error_pool(desc.lod_error, world_center_self, scale);
        let parent_err_px = lod_pixel_error_pool(parent.lod_error, world_center_parent, scale);
        if (!(my_err_px <= params.lod_target_error_pixels
            && parent_err_px > params.lod_target_error_pixels))
        {
            return;
        }
    }

    let world_center = (inst.transform * vec4<f32>(desc.bounds_center, 1.0)).xyz;
    if (sphere_outside_frustum(world_center, desc.bounding_radius)) {
        return;
    }

    let world_apex = (inst.transform * vec4<f32>(desc.cone_apex, 1.0)).xyz;
    let world_axis = normalize(
        (inst.transform * vec4<f32>(desc.cone_axis, 0.0)).xyz
    );
    if (camera_in_cone(world_apex, world_axis, desc.cone_cutoff)) {
        return;
    }

    // Pack: instance_id (high 16) | global_meshlet_idx (low 16).
    // Caps: 64K instances, 64K meshlets in the entire pool.
    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = (instance_id << 16u) | (global_meshlet_idx & 0xffffu);
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_scene_pool(@builtin(global_invocation_id) gid: vec3<u32>) {
    run_cull_scene_pool(gid.x);
}
