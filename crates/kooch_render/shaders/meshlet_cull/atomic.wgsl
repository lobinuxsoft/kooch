
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

// #454.4 — per-thread reject-reason tag buffer. One u32 slot per cull
// thread (= instance_count × meshlets_per_mesh, sized in lock-step
// with visible_meshlets). Values:
//   0 = thread skipped (out of bounds for instance's mesh meshlet
//       count, or for total_threads).
//   1 = passed every cull stage and was emitted into visible_meshlets.
//   2 = rejected by frustum.
//   3 = rejected by backface cone.
//   4 = rejected by Hi-Z occlusion (pass A only — the Hi-Z 2-pass
//       entry doesn't write reject_reasons in #454.4 scope; reserved
//       for the follow-up that wires it).
// All writes are gated to `params.debug_active != 0`. Production
// rendering pays exactly one uniform compare per thread + zero
// stores. The buffer is cleared by the dispatcher at frame start so
// stale entries from the previous frame never bleed through.
@group(4) @binding(0) var<storage, read_write> reject_reasons: array<u32>;

// #454.6 — per-stage cull survivor counters. AtomicAdded at each
// stage tail when `params.debug_active != 0`. Slot layout:
//   [0] = after_frustum   (passed frustum test)
//   [1] = after_backface  (passed frustum + backface)
//   [2] = after_hi_z      (only the Hi-Z 2-pass entry writes here;
//         this entry leaves it 0 because it doesn't run an Hi-Z test)
//   [3] = total_visible   (terminal — equals visible_count)
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
// LOD selector dropped the meshlet (above_too_coarse / below_fine
// / debug-mode override). Same colour as "skipped" in the overlay
// LUT today; surfaced separately so the debug HUD can split LOD
// drops from genuinely out-of-range threads later.
const REJECT_REASON_LOD: u32 = 5u;

fn record_reject(thread_id: u32, reason: u32) {
    if (params.debug_active != 0u) {
        reject_reasons[thread_id] = reason;
    }
}

// AABB-vs-frustum (positive-vertex test). Ports atomic_hi_z's
// `aabb_outside_frustum_atomic` to the non-Hi-Z R64 path so both
// entries reject identically. Sphere-bounds + plane-distance left
// silhouette holes at viewport edges where projected AABBs
// partially leave the frustum (#488 documented this fix for the
// Hi-Z path; the R64 path inherits it here).
//
// Drops the far plane on purpose (5 planes: 4 lateral + ndc.z >= 0).
// Meshlets straddling the near plane stay in — the rasterizer clips
// them naturally, and rejecting at the cull would re-introduce the
// same silhouette holes the AABB switch was meant to close.
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

@compute @workgroup_size(64, 1, 1)
fn cs_lod_compute_group_max_err(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let thread_id = linear_thread(gid, groups);
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
    // Out-of-dispatch threads have no reject_reasons[] slot to claim
    // (the buffer is sized to total_threads). Bail before touching it.
    if (thread_id >= total_threads) {
        return;
    }
    let instance_id = thread_id / max_meshlets;
    let meshlet_offset = thread_id % max_meshlets;

    let inst = instances[instance_id];
    let mesh_desc = pool_mesh_descriptors[inst.mesh_id];
    if (meshlet_offset >= mesh_desc.meshlet_count) {
        record_reject(thread_id, REJECT_REASON_SKIPPED);
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
    run_cull_scene_pool_atomic(linear_thread(gid, groups));
}
