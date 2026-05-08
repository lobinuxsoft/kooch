
// ---------------------------------------------------------------
// #465 — group-atomic LOD descent (2-pass cull).
//
// Pass 1 (cs_lod_compute_group_max_err) computes, per group_id, the
// maximum pixel-projected error among the parents of that group. The
// "parents of group G" are the meshlets that the children in G point
// at via parent_meshlet_index — but iterating children is more
// convenient because each child knows its group_index and its
// parent. So per child C, contribute pixel_err(C.parent) to
// group_max_err[C.group_index] via atomicMax. Sibling children all
// converge their group's slot to max(parent_err over all parents of
// the group), which is what pass 2 needs.
//
// f32 → u32 bitcast for the atomic: pixel errors are non-negative,
// so the IEEE-754 bit pattern preserves ordering for atomicMax. The
// CPU-side group_max_err buffer is cleared to 0 (= 0.0 f32) each
// frame.
//
// Pass 2 (cs_cull_scene_pool_atomic) selects atomically:
//   above_too_coarse = (M.group_index == NONE) ? true
//                       : group_max_err[M.group_index] > threshold
//   below_fine       = (M.children_group_index == NONE) ? true
//                       : group_max_err[M.children_group_index] <= threshold
//   render M iff (above_too_coarse && below_fine && passes_frustum_cone)
//
// The atomicity invariant: every meshlet sharing a group_index
// produces the SAME above_too_coarse decision (they read the same
// slot). So sibling children either all descend together or all
// stay together — no half-descended group → no torn coverage seam.
// ---------------------------------------------------------------

@group(3) @binding(0) var<storage, read_write> group_max_err: array<atomic<u32>>;

fn lod_pixel_error_world_pool(lod_error: f32, world_center: vec3<f32>) -> f32 {
    let to_cam = world_center - params.camera_position;
    let dist = max(length(to_cam), 0.0001);
    return lod_error * params.lod_error_to_pixel_factor / dist;
}

@compute @workgroup_size(64, 1, 1)
fn cs_lod_compute_group_max_err(@builtin(global_invocation_id) gid: vec3<u32>) {
    let max_meshlets = scene_params.meshlets_per_mesh;
    let total_threads = scene_params.instance_count * max_meshlets;
    if (gid.x >= total_threads) {
        return;
    }
    let instance_id = gid.x / max_meshlets;
    let meshlet_offset = gid.x % max_meshlets;

    let inst = instances[instance_id];
    let mesh_desc = pool_mesh_descriptors[inst.mesh_id];
    if (meshlet_offset >= mesh_desc.meshlet_count) {
        return;
    }

    let global_meshlet_idx = mesh_desc.first_meshlet + meshlet_offset;
    let m = pool_meshlets[global_meshlet_idx];

    // Roots and meshlets without an above-group don't contribute —
    // there's no group_max_err slot for "no group".
    if (m.parent_meshlet_index == 0xFFFFFFFFu) {
        return;
    }
    if (m.group_index == 0xFFFFFFFFu) {
        return;
    }

    let parent = pool_meshlets[m.parent_meshlet_index];
    let world_parent_center =
        (inst.transform * vec4<f32>(parent.bounds_center, 1.0)).xyz;
    let parent_err_px =
        lod_pixel_error_world_pool(parent.lod_error, world_parent_center);

    // bitcast preserves ordering for non-negative IEEE-754 floats.
    let parent_err_bits = bitcast<u32>(max(parent_err_px, 0.0));
    // Per-instance slot: m.group_index was pool-shifted by
    // mesh_desc.group_base at register(); subtract to recover the
    // mesh-local id, then offset by inst.group_base so each instance
    // owns a disjoint slot range. Without this every instance of the
    // mesh atomicMaxes into the same slot and pass 2 collapses every
    // instance's LOD to the closest one's verdict (#474).
    let local_group = m.group_index - mesh_desc.group_base;
    let slot = inst.group_base + local_group;
    atomicMax(&group_max_err[slot], parent_err_bits);
}

fn run_cull_scene_pool_atomic(thread_id: u32) {
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
    let m = pool_meshlets[global_meshlet_idx];

    // Per-instance LOD level lock (#467). When the editor's LOD-
    // stack inspector spawns ghost copies of an entity, each copy
    // sets `lod_force_level >= 0` to render only its own slice of
    // the chain. Short-circuits both the debug-mode overrides and
    // the normal selector below.
    if (inst.lod_force_level >= 0) {
        if (i32(m.lod_level) != inst.lod_force_level) {
            return;
        }
    } else if (params.debug_mode == 8u) {
        // 8 = OnlyLod0 → emit iff lod_error == 0.0
        if (m.lod_error != 0.0) {
            return;
        }
    } else if (params.debug_mode == 9u) {
        // 9 = OnlyRoots → emit iff parent_meshlet_index == sentinel
        if (m.parent_meshlet_index != 0xFFFFFFFFu) {
            return;
        }
    } else {
        // Normal group-atomic descent decisions.
        let target_px = params.lod_target_error_pixels;

        var above_too_coarse: bool;
        if (m.group_index == 0xFFFFFFFFu) {
            // Root or no above-group → above is trivially "too
            // coarse" so the meshlet is the only available level.
            above_too_coarse = true;
        } else {
            // See pass 1: per-instance slot decoding (#474).
            let local_group = m.group_index - mesh_desc.group_base;
            let slot = inst.group_base + local_group;
            let bits = atomicLoad(&group_max_err[slot]);
            let group_err_px = bitcast<f32>(bits);
            above_too_coarse = group_err_px > target_px;
        }

        var below_fine: bool;
        if (m.children_group_index == 0xFFFFFFFFu) {
            // LOD 0 or no children → no further descent possible,
            // this level is the floor.
            below_fine = true;
        } else {
            let local_group = m.children_group_index - mesh_desc.group_base;
            let slot = inst.group_base + local_group;
            let bits = atomicLoad(&group_max_err[slot]);
            let group_err_px = bitcast<f32>(bits);
            below_fine = group_err_px <= target_px;
        }

        if (!(above_too_coarse && below_fine)) {
            return;
        }
    }

    // Frustum + cone tests, then emit.
    let world_center = (inst.transform * vec4<f32>(m.bounds_center, 1.0)).xyz;
    if (sphere_outside_frustum(world_center, m.bounding_radius)) {
        return;
    }

    let world_apex = (inst.transform * vec4<f32>(m.cone_apex, 1.0)).xyz;
    let world_axis = normalize(
        (inst.transform * vec4<f32>(m.cone_axis, 0.0)).xyz
    );
    if (camera_in_cone(world_apex, world_axis, m.cone_cutoff)) {
        return;
    }

    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = (instance_id << 16u) | (global_meshlet_idx & 0xffffu);
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_scene_pool_atomic(@builtin(global_invocation_id) gid: vec3<u32>) {
    run_cull_scene_pool_atomic(gid.x);
}
