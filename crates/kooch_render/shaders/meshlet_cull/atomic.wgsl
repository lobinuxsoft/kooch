
// Pass 1 (cs_lod_compute_group_max_err) computes, per group_id, the maximum pixel-projected error
// among the parents of that group.

@group(3) @binding(0) var<storage, read_write> group_max_err: array<atomic<u32>>;

// thread (= instance_count × meshlets_per_mesh, sized in lock-step with visible_meshlets).
@group(4) @binding(0) var<storage, read_write> reject_reasons: array<u32>;

// stage tail when `params.debug_active != 0`.
@group(4) @binding(1) var<storage, read_write> stage_counters: array<atomic<u32>, 4>;

const STAGE_AFTER_FRUSTUM: u32 = 0u;
const STAGE_AFTER_BACKFACE: u32 = 1u;
const STAGE_AFTER_HI_Z: u32 = 2u;
const STAGE_TOTAL_VISIBLE: u32 = 3u;

fn record_stage_survivor(stage: u32) {
    if (params.debug_active != 0u) {
        atomicAdd(&stage_counters[stage], 1u);
    }
}

const REJECT_REASON_SKIPPED: u32 = 0u;
const REJECT_REASON_PASSED: u32 = 1u;
const REJECT_REASON_FRUSTUM: u32 = 2u;
const REJECT_REASON_BACKFACE: u32 = 3u;
// LOD selector dropped the meshlet (above_too_coarse / below_fine / debug-mode override). Same
// colour as "skipped" in the overlay LUT today; surfaced separately so the debug HUD can split LOD
// drops from genuinely out-of-range threads later.
const REJECT_REASON_LOD: u32 = 5u;
// became meshlets. Written at the instance's first rectangle slot, so the overlay draws one box per
// rejected instance rather than one per meshlet it never expanded.
const REJECT_REASON_INSTANCE: u32 = 6u;
// Same pass, the other test: the instance projects to fewer pixels than `params.min_screen_pixels`.
// Separate from the frustum reject because "off screen" and "too small to matter" are different
// answers and only one of them is a setting.
const REJECT_REASON_REACH: u32 = 7u;

fn record_reject(thread_id: u32, reason: u32) {
    if (params.debug_active != 0u) {
        reject_reasons[thread_id] = reason;
    }
}

// AABB-vs-frustum (positive-vertex test). Ports atomic_hi_z's `aabb_outside_frustum_atomic` to the
// non-Hi-Z R64 path so both entries reject identically.
fn aabb_outside_frustum_local(
    world_from_local: mat4x4<f32>,
    aabb_min_local: vec3<f32>,
    aabb_max_local: vec3<f32>,
) -> bool {
    let center = (aabb_min_local + aabb_max_local) * 0.5;
    let half_extent = (aabb_max_local - aabb_min_local) * 0.5;
    let clip_from_local = params.view_proj * world_from_local;
    let row_major = transpose(clip_from_local);
    let planes = array<vec4<f32>, 5>(
        row_major[3] + row_major[0],
        row_major[3] - row_major[0],
        row_major[3] + row_major[1],
        row_major[3] - row_major[1],
        row_major[2],
    );
    for (var i = 0u; i < 5u; i = i + 1u) {
        let plane = planes[i] / length(planes[i].xyz);
        let flipped = half_extent * sign(plane.xyz);
        if (dot(center + flipped, plane.xyz) <= -plane.w) {
            return true;
        }
    }
    return false;
}


fn lod_pixel_error_world_pool(lod_error: f32, world_center: vec3<f32>, world_scale: f32) -> f32 {
    // 🔴 Orthographic views do not shrink error with distance.
    let world_error = lod_error * world_scale;
    if (params.lod_orthographic == 1u) {
        return world_error * params.lod_error_to_pixel_factor;
    }
    let to_cam = world_center - params.camera_position;
    let dist = max(length(to_cam), 0.0001);
    return world_error * params.lod_error_to_pixel_factor / dist;
}

// Pass 1's body, once the thread knows WHICH meshlet it owns. Split out so the two-level cull
// (#1002) can reach the same reduction from a chunk instead of from a rectangle index — the
// decision has to be identical in both passes or a group descends half-way.
fn lod_group_max_err(instance_id: u32, meshlet_offset: u32) {
    let inst = instances[instance_id];
    let mesh_desc = pool_mesh_descriptors[inst.mesh_id];
    if (meshlet_offset >= mesh_desc.meshlet_count || skipped(inst.flags)) {
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
        lod_pixel_error_world_pool(
            parent.lod_error,
            world_parent_center,
            instance_world_scale(inst.transform),
        );

    // bitcast preserves ordering for non-negative IEEE-754 floats.
    let parent_err_bits = bitcast<u32>(max(parent_err_px, 0.0));
    // Per-instance slot: m.group_index was pool-shifted by mesh_desc.group_base at register();
    // subtract to recover the mesh-local id, then offset by inst.group_base so each instance owns a
    // disjoint slot range.
    let local_group = m.group_index - mesh_desc.group_base;
    let slot = inst.group_base + local_group;
    atomicMax(&group_max_err[slot], parent_err_bits);
}

@compute @workgroup_size(64, 1, 1)
fn cs_lod_compute_group_max_err(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let thread_id = linear_thread(gid, groups);
    let max_meshlets = scene_params.meshlets_per_mesh;
    if (thread_id >= scene_params.instance_count * max_meshlets) {
        return;
    }
    lod_group_max_err(thread_id / max_meshlets, thread_id % max_meshlets);
}

// Pass 2's body, once the thread knows WHICH meshlet it owns.
fn cull_pool_atomic(thread_id: u32, instance_id: u32, meshlet_offset: u32) {
    let inst = instances[instance_id];
    let mesh_desc = pool_mesh_descriptors[inst.mesh_id];
    if (meshlet_offset >= mesh_desc.meshlet_count || skipped(inst.flags)) {
        record_reject(thread_id, REJECT_REASON_SKIPPED);
        return;
    }

    let global_meshlet_idx = mesh_desc.first_meshlet + meshlet_offset;
    let m = pool_meshlets[global_meshlet_idx];

    // Per-instance LOD level lock (#467). When the editor's LOD- stack inspector spawns ghost
    // copies of an entity, each copy sets `lod_force_level >= 0` to render only its own slice of
    // the chain. Short-circuits both the debug-mode overrides and the normal selector below.
    if (inst.lod_force_level >= 0) {
        if (i32(m.lod_level) != inst.lod_force_level) {
            record_reject(thread_id, REJECT_REASON_LOD);
            return;
        }
    } else if (params.debug_mode == 8u) {
        // 8 = OnlyLod0 → emit iff lod_error == 0.0
        if (m.lod_error != 0.0) {
            record_reject(thread_id, REJECT_REASON_LOD);
            return;
        }
    } else if (params.debug_mode == 9u) {
        // 9 = OnlyRoots → emit iff parent_meshlet_index == sentinel
        if (m.parent_meshlet_index != 0xFFFFFFFFu) {
            record_reject(thread_id, REJECT_REASON_LOD);
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
            record_reject(thread_id, REJECT_REASON_LOD);
            return;
        }
    }

    // AABB-vs-frustum: tighter than sphere bounds; closes silhouette
    // holes at viewport edges (#488 parity for the R64 path).
    if (aabb_outside_frustum_local(inst.transform, m.aabb_min, m.aabb_max)) {
        record_reject(thread_id, REJECT_REASON_FRUSTUM);
        return;
    }
    record_stage_survivor(STAGE_AFTER_FRUSTUM);

    let world_apex = (inst.transform * vec4<f32>(m.cone_apex, 1.0)).xyz;
    let world_axis = normalize(
        (inst.transform * vec4<f32>(m.cone_axis, 0.0)).xyz
    );
    if (camera_in_cone(world_apex, world_axis, m.cone_cutoff)) {
        record_reject(thread_id, REJECT_REASON_BACKFACE);
        return;
    }
    record_stage_survivor(STAGE_AFTER_BACKFACE);

    record_reject(thread_id, REJECT_REASON_PASSED);
    record_stage_survivor(STAGE_TOTAL_VISIBLE);
    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = (instance_id << 16u) | (global_meshlet_idx & 0xffffu);
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_scene_pool_atomic(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let thread_id = linear_thread(gid, groups);
    let max_meshlets = scene_params.meshlets_per_mesh;
    // Out-of-dispatch threads have no reject_reasons[] slot to claim
    // (the buffer is sized to total_threads). Bail before touching it.
    if (thread_id >= scene_params.instance_count * max_meshlets) {
        return;
    }
    cull_pool_atomic(thread_id, thread_id / max_meshlets, thread_id % max_meshlets);
}
