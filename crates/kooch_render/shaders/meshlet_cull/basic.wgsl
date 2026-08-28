
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
fn cs_cull(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    run_cull_basic(linear_thread(gid, groups));
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_hi_z(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    run_cull_with_hi_z(linear_thread(gid, groups));
}
