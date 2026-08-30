
// ---------------------------------------------------------------
// Phase 1.E.1 — scene-wide cull. Instance buffer + per-instance
// transform applied to per-meshlet bounds. ONE dispatch enumerates
// (instance, meshlet) pairs across the whole scene.
// ---------------------------------------------------------------

struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    // i32 stored as u32 over the wire — bitcast<i32> in the shader.
    // < 0 (LOD_FORCE_NONE = i32::MIN sentinel) = normal selector;
    // ≥ 0 = render only meshlets with lod_level == this value (#467
    // LOD-stack inspector).
    lod_force_level: i32,
    // Per-instance prefix-sum base into `group_max_err`. Pass 1 / pass 2
    // both compute `slot = group_base + (m.group_index - mesh_desc.group_base)`
    // so two instances of the same mesh write to disjoint slot ranges
    // and pick LOD independently (#474). 0 is valid when the scene has
    // at most one instance per mesh.
    group_base: u32,
    // #804 — per-instance bits; bit 0 is "receives shadows". Was
    // `_pad0`, so the 96-byte stride is unchanged.
    flags: u32,
    _pad1: u32,
    _pad2: u32,
}

struct SceneCullParams {
    instance_count: u32,
    meshlets_per_mesh: u32,
    // The scene's real LOD-group count. Read by the CPU to size the
    // group-error arenas, never by a shader — it is here because this
    // is the struct that already reaches everything that needs it.
    group_capacity: u32,
    // Chunk slots the two-level cull's list holds (#1002). A CAPACITY,
    // not a count: `cs_cull_instances` reserves with an atomic that is
    // never clamped, so every reader clamps to this instead. Was
    // `_pad1`, so the 16-byte layout is unchanged.
    chunk_capacity: u32,
}

@group(2) @binding(0) var<storage, read> instances: array<MeshInstance>;
@group(2) @binding(1) var<uniform> scene_params: SceneCullParams;

// Projects a meshlet's world-space LOD error to pixels at the camera.
// Standard perspective formula: `error * scale_y * viewport_h / (2 * dist)`,
// where `scale_y = 1 / tan(fovy/2)`. The CPU side rolls
// `0.5 * viewport_h * scale_y` into `lod_error_to_pixel_factor`.

fn lod_pixel_error(lod_error: f32, world_center: vec3<f32>, world_scale: f32) -> f32 {
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

fn run_cull_scene(thread_id: u32) {
    let total_threads = scene_params.instance_count * scene_params.meshlets_per_mesh;
    if (thread_id >= total_threads) {
        return;
    }
    let instance_id = thread_id / scene_params.meshlets_per_mesh;
    let meshlet_idx = thread_id % scene_params.meshlets_per_mesh;

    // params.meshlet_count carries the registered mesh's actual meshlet
    // count — the dispatch uses meshlets_per_mesh as the worst-case
    // stride (e.g. for 1.E.1b's mixed-mesh pool); per-thread bounds-
    // check ensures we don't read past the descriptor array.
    if (meshlet_idx >= params.meshlet_count) {
        return;
    }

    let inst = instances[instance_id];
    let desc = descriptors[meshlet_idx];

    // Continuous-LOD selection (#442). For each meshlet:
    //   - Roots (parent == 0xFFFFFFFFu) always pass — there is no
    //     coarser option to descend from. Single-LOD assets land here
    //     for every meshlet, preserving the legacy behaviour.
    //   - Non-roots pass only when their own pixel error is at or below
    //     the target AND their parent's pixel error is above the
    //     target. That is the cluster-DAG "boundary" rule: pick the
    //     finest level whose parent is too coarse.
    if (desc.parent_meshlet_index != 0xFFFFFFFFu) {
        let parent = descriptors[desc.parent_meshlet_index];
        let world_center_self = (inst.transform * vec4<f32>(desc.bounds_center, 1.0)).xyz;
        let world_center_parent = (inst.transform * vec4<f32>(parent.bounds_center, 1.0)).xyz;
        let scale = instance_world_scale(inst.transform);
        let my_err_px = lod_pixel_error(desc.lod_error, world_center_self, scale);
        let parent_err_px = lod_pixel_error(parent.lod_error, world_center_parent, scale);
        if (!(my_err_px <= params.lod_target_error_pixels
            && parent_err_px > params.lod_target_error_pixels))
        {
            return;
        }
    }

    // Transform per-meshlet bounds to world space via the instance's
    // transform. Uniform-scale assumption keeps `bounding_radius`
    // reusable as-is; non-uniform scale support is a 1.E follow-up
    // (compute max-component scale once, cache on `MeshInstance`).
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

    // Pack: instance_id (high 16 bits) | meshlet_idx (low 16 bits).
    // Caps: 64K instances, 64K meshlets per mesh.
    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = (instance_id << 16u) | (meshlet_idx & 0xffffu);
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_scene(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    run_cull_scene(linear_thread(gid, groups));
}
