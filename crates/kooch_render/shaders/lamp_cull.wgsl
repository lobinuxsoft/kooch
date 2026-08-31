// lamp_cull.wgsl — one hierarchical cull for every lamp (#939).
//
// CONCATENATED after `cluster_common.wgsl` and `page_table.wgsl`.
//
// # Why lamps get their own machine
//
// A survivor list is a LOD picked for a VIEW. The sun's seventeen
// lists are picked for orthographic boxes centred on the camera, and
// lamps briefly borrowed them — a close lamp's casters fell outside
// the fine levels' box and its shadow vanished, a coarse bucket handed
// root meshlets. The first fix was the retired cube path's recipe run
// literally: one `MeshletCull` per lamp, dispatched from the CPU,
// capped at 32. Correct, and the wrong shape — cost `lamps × scene`
// plus a CPU loop per view.
//
// This file is Olsson et al. 2014 §3.4/§5.2 adapted to the meshlet
// pool: the scene already has the two hierarchy levels the paper
// builds (instances over meshlets), so the cull queries them instead
// of walking everything.
//
// | pass | domain | what it decides |
// |---|---|---|
// | `cs_lamp_pairs` | lights × instances | which instances a light's range reaches |
// | `cs_lamp_args`  | 1 thread | sizes the meshlet-domain dispatches from the pair count |
// | `cs_lamp_err`   | pairs × meshlets | parent pixel error per (lamp, group) — the #465 reduction, all lamps at once |
// | `cs_lamp_cull`  | pairs × meshlets | group-coherent LOD cut + range + cone; survivors into the lamp's slice |
//
// # 🔴 One group-error arena for every lamp
//
// The group-coherent descent (#465) is why the module doc once said a
// per-lamp cull "cannot simply be inlined": the reduction is per view.
// It CAN be one dispatch — the arena is indexed
// `[slot * group_capacity + group]`, so every lamp's reduction runs in
// the same pass and sibling meshlets of one lamp still read the same
// slot. The memory is the price: `LAMP_CULLS × group_capacity × 4 B`,
// which is the stated ceiling on `LAMP_CULLS`.
//
// # 🔴 Fixed survivor slices, no scan
//
// The paper counts, prefix-sums and emits. A fixed `LAMP_SURVIVORS`
// slice per lamp drops both extra passes and the double test: one
// atomicAdd names the slot. The count is written uncapped so an
// overflowing lamp is visible in the counters; every reader clamps.
//
// # View independence
//
// Nothing here reads a camera: the frustum is the light's own range
// and the LOD is measured from the light's position. The whole set of
// passes runs ONCE per frame and both editor views consume the same
// survivors — where the sun's culls run per view because their boxes
// follow the eye.

struct LampMeshDescriptor {
    first_meshlet: u32,
    meshlet_count: u32,
    vertex_offset: u32,
    meshlet_vertex_offset: u32,
    meshlet_triangle_offset: u32,
    group_base: u32,
    group_count: u32,
    _pad0: u32,
}

// Stride 96 B, mirroring the cull side — see `page_expand.wgsl` for
// the precedent and the warning about a mismatch.
struct LampMeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    flags: u32,
    _pad1: u32,
    _pad2: u32,
}

struct LampMeshlet {
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

struct LampCullUniform {
    // x instances, y meshlets_per_mesh (the pool-wide max), z lamp
    // slots this frame (`min(light_count, LAMP_CULLS)`), w pair cap.
    scene: vec4<u32>,
    // x group_capacity (slots per lamp in the error arena), y the sun's
    // bucket count (`chain.x` — survivors land at `[y + slot]` of the
    // raster's `visible_counts`), z/w unused.
    arena: vec4<u32>,
    // x the LOD target in texels, y the error-to-texel factor
    // (`0.5 * LOCAL_MAX_TEXELS`, 90° faces make `proj_scale_y` 1).
    lod: vec4<f32>,
}

@group(0) @binding(0) var<uniform> lamp: LampCullUniform;
@group(0) @binding(1) var<storage, read> lamp_lights: array<ClusterLight>;
// (center, radius) per MESH, parallel to the descriptors — see
// `GpuGlobalMeshPool::mesh_bounds`.
@group(0) @binding(2) var<storage, read> lamp_mesh_bounds: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read> lamp_instances: array<LampMeshInstance>;
// Flat words, because WGSL forbids an atomic inside a vector:
// `[0]` the pair count, `[1]` the pairs dropped over the cap, then
// two words per pair — the lamp slot and the instance.
@group(0) @binding(4) var<storage, read_write> lamp_pairs: array<atomic<u32>>;
@group(0) @binding(5) var<storage, read_write> lamp_args: array<u32>;
@group(0) @binding(6) var<storage, read> lamp_mesh_descriptors: array<LampMeshDescriptor>;
@group(0) @binding(7) var<storage, read> lamp_meshlets: array<LampMeshlet>;
// `[slot * group_capacity + group]`: max parent pixel error, f32 bits.
@group(0) @binding(8) var<storage, read_write> lamp_group_err: array<atomic<u32>>;
// `[slot * LAMP_SURVIVORS ..]`: the lamp's packed survivors.
@group(0) @binding(9) var<storage, read_write> lamp_survivors: array<u32>;
// The page raster's `visible_counts`, written at `[chain.x + slot]` —
// the same words `cs_expand_args` sizes the expansion from, so a
// lamp's survivors need no copy to be consumed. Uncapped; readers
// clamp to `LAMP_SURVIVORS`.
@group(0) @binding(10) var<storage, read_write> lamp_counts: array<atomic<u32>>;

const LAMP_GROUP: u32 = 64u;

/// Largest axis scale of an instance's transform. Mirrors
/// `instance_world_scale` in `meshlet_cull/common.wgsl`.
fn lamp_world_scale(transform: mat4x4<f32>) -> f32 {
    return max(
        length(transform[0].xyz),
        max(length(transform[1].xyz), length(transform[2].xyz)),
    );
}

/// A simplification error in texels of the lamp's finest face, from
/// the light's own position — the perspective form, because a lamp has
/// a real viewpoint and error really does shrink with distance.
fn lamp_pixel_error(lod_error: f32, world_center: vec3<f32>, world_scale: f32, eye: vec3<f32>) -> f32 {
    let world_error = lod_error * world_scale;
    let dist = max(length(world_center - eye), 0.0001);
    return world_error * lamp.lod.y / dist;
}

// Which instances a light's range reaches. Sphere against sphere: the
// mesh bounds survive an arbitrary rotation where an AABB would need
// rebuilding, which is why the pool stores them as spheres (#847).
@compute @workgroup_size(LAMP_GROUP, 1, 1)
fn cs_lamp_pairs(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let instances = lamp.scene.x;
    // Tiled like the two passes below: this product is lamps times
    // instances and passes the one-dimension limit at 4.2 M.
    let thread = lamp_thread(gid, groups);
    if thread >= lamp.scene.z * instances {
        return;
    }
    let slot = thread / instances;
    let instance = thread % instances;
    let light = lamp_lights[slot];
    // Directional lights are a prefix of the buffer and never mark a
    // local page; their slots stay empty rather than shifting everyone
    // else's bucket.
    if light.kind == 0u || light.range <= 0.0 {
        return;
    }
    let inst = lamp_instances[instance];
    let bounds = lamp_mesh_bounds[inst.mesh_id];
    let centre = (inst.transform * vec4<f32>(bounds.xyz, 1.0)).xyz;
    let radius = bounds.w * lamp_world_scale(inst.transform);
    if distance(centre, light.position) > light.range + radius {
        return;
    }
    let index = atomicAdd(&lamp_pairs[0], 1u);
    if index >= lamp.scene.w {
        atomicAdd(&lamp_pairs[1], 1u);
        return;
    }
    atomicStore(&lamp_pairs[2u + index * 2u], slot);
    atomicStore(&lamp_pairs[3u + index * 2u], instance);
}

// Workgroups one dimension of a dispatch may hold. Mirrors
// `MAX_WORKGROUPS_PER_DIM` in `kooch_core::gpu::limits` — the Vulkan,
// D3D12 and Metal floor, and what desktop adapters actually report.
const LAMP_DIM_LIMIT: u32 = 65535u;

// The meshlet-domain dispatch size: pairs times the pool-wide meshlet
// max, a number that only exists on the GPU once the pairs are counted.
//
// # 🔴 TWO dimensions, because one cannot hold it
//
// `lamp.scene.y` is `meshlets_per_mesh`, and that is the maximum over
// the WHOLE scene rather than this mesh's own count — so the product is
// `pairs * scene_max`, and it passes 65 535 workgroups long before a
// scene is interesting. Measured on `dense.scene`: 2157 instances at a
// scene max of 4563 meshlets, 64 lamps, so even the old cap of 16 384
// pairs asks for 1.17 MILLION workgroups. Eighteen times the limit.
//
// An indirect dispatch past `maxComputeWorkGroupCount` is undefined, and
// what it does here is nothing: the two heavy passes never ran, every
// lamp bucket kept zero survivors, and every lamp page was stamped empty
// and cleared. The reader then answers "nothing occludes" over a page
// that is resident, correctly keyed and perfectly blank — no lamp in the
// scene cast a shadow, with every counter reading healthy.
//
// The engine already has the shape for this: `tiled_workgroups` spills
// the excess into `y` and the shader re-linearises from
// `num_workgroups.x`. Its own comment says why clamping is not the
// alternative — the scene renders with geometry missing and it reads as
// a bug in the LOD chain. That is exactly what happened.
//
// ⚠️ This makes the dispatch legal, not small. `pairs * scene_max` is
// still overwhelmingly empty threads, and the fix for that is the
// chunking the camera's cull already has (`chunks_for`).
@compute @workgroup_size(1, 1, 1)
fn cs_lamp_args() {
    let pairs = min(atomicLoad(&lamp_pairs[0]), lamp.scene.w);
    let threads = pairs * lamp.scene.y;
    let groups = max((threads + LAMP_GROUP - 1u) / LAMP_GROUP, 1u);
    if groups <= LAMP_DIM_LIMIT {
        lamp_args[0] = groups;
        lamp_args[1] = 1u;
    } else {
        lamp_args[0] = LAMP_DIM_LIMIT;
        lamp_args[1] = (groups + LAMP_DIM_LIMIT - 1u) / LAMP_DIM_LIMIT;
    }
    lamp_args[2] = 1u;
}

// The linear thread index of a dispatch that may be two-dimensional.
//
// Mirrors `tiled_workgroups`: `x` saturates at the limit and `y` carries
// the rest, so the row stride is the whole `x` extent in THREADS.
fn lamp_thread(gid: vec3<u32>, groups: vec3<u32>) -> u32 {
    return gid.y * (groups.x * LAMP_GROUP) + gid.x;
}

// The #465 reduction, every lamp at once: per child, contribute the
// PARENT's pixel error to the child's group slot, in this lamp's own
// row of the arena. Siblings of one lamp converge one slot, so pass
// two's descent decision is coherent per lamp — no torn seam, no hole
// in a caster.
@compute @workgroup_size(LAMP_GROUP, 1, 1)
fn cs_lamp_err(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let per_mesh = lamp.scene.y;
    let pairs = min(atomicLoad(&lamp_pairs[0]), lamp.scene.w);
    let thread = lamp_thread(gid, groups);
    if thread >= pairs * per_mesh {
        return;
    }
    let pair = thread / per_mesh;
    let offset = thread % per_mesh;
    let slot = atomicLoad(&lamp_pairs[2u + pair * 2u]);
    let instance = atomicLoad(&lamp_pairs[3u + pair * 2u]);

    let inst = lamp_instances[instance];
    let mesh = lamp_mesh_descriptors[inst.mesh_id];
    if offset >= mesh.meshlet_count {
        return;
    }
    let m = lamp_meshlets[mesh.first_meshlet + offset];
    if m.parent_meshlet_index == 0xFFFFFFFFu || m.group_index == 0xFFFFFFFFu {
        return;
    }
    let parent = lamp_meshlets[m.parent_meshlet_index];
    let centre = (inst.transform * vec4<f32>(parent.bounds_center, 1.0)).xyz;
    let err = lamp_pixel_error(
        parent.lod_error,
        centre,
        lamp_world_scale(inst.transform),
        lamp_lights[slot].position,
    );
    let group = inst.group_base + (m.group_index - mesh.group_base);
    atomicMax(
        &lamp_group_err[slot * lamp.arena.x + group],
        bitcast<u32>(max(err, 0.0)),
    );
}

// The cull itself: group-coherent LOD cut, range, backface cone — the
// same tests the retired cube path ran per light, in one dispatch for
// all of them.
@compute @workgroup_size(LAMP_GROUP, 1, 1)
fn cs_lamp_cull(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let per_mesh = lamp.scene.y;
    let pairs = min(atomicLoad(&lamp_pairs[0]), lamp.scene.w);
    let thread = lamp_thread(gid, groups);
    if thread >= pairs * per_mesh {
        return;
    }
    let pair = thread / per_mesh;
    let offset = thread % per_mesh;
    let slot = atomicLoad(&lamp_pairs[2u + pair * 2u]);
    let instance = atomicLoad(&lamp_pairs[3u + pair * 2u]);

    let inst = lamp_instances[instance];
    let mesh = lamp_mesh_descriptors[inst.mesh_id];
    if offset >= mesh.meshlet_count {
        return;
    }
    let global_index = mesh.first_meshlet + offset;
    let m = lamp_meshlets[global_index];
    let light = lamp_lights[slot];

    // Group-coherent descent — mirrors `cs_cull_scene_pool_atomic`.
    let lod_target = lamp.lod.x;
    var above_too_coarse = true;
    if m.group_index != 0xFFFFFFFFu {
        let group = inst.group_base + (m.group_index - mesh.group_base);
        let bits = atomicLoad(&lamp_group_err[slot * lamp.arena.x + group]);
        above_too_coarse = bitcast<f32>(bits) > lod_target;
    }
    var below_fine = true;
    if m.children_group_index != 0xFFFFFFFFu {
        let group = inst.group_base + (m.children_group_index - mesh.group_base);
        let bits = atomicLoad(&lamp_group_err[slot * lamp.arena.x + group]);
        below_fine = bitcast<f32>(bits) <= lod_target;
    }
    if !(above_too_coarse && below_fine) {
        return;
    }

    let scale = lamp_world_scale(inst.transform);
    let centre = (inst.transform * vec4<f32>(m.bounds_center, 1.0)).xyz;
    let radius = m.bounding_radius * scale;
    if distance(centre, light.position) > light.range + radius {
        return;
    }

    // Backface cone from the light's position — a lamp has a real
    // viewpoint, which is the condition `camera_in_cone` demands.
    if m.cone_cutoff < 1.0 {
        let apex = (inst.transform * vec4<f32>(m.cone_apex, 1.0)).xyz;
        let axis = normalize((inst.transform * vec4<f32>(m.cone_axis, 0.0)).xyz);
        let to_apex = apex - light.position;
        let len_sq = dot(to_apex, to_apex);
        if len_sq > 0.0 && dot(to_apex / sqrt(len_sq), axis) >= m.cone_cutoff {
            return;
        }
    }

    let count = atomicAdd(&lamp_counts[lamp.arena.y + slot], 1u);
    if count >= LAMP_SURVIVORS {
        // The count keeps climbing past the slice on purpose — the
        // overflow is the counter.
        return;
    }
    lamp_survivors[slot * LAMP_SURVIVORS + count] =
        (instance << 16u) | (global_index & 0xffffu);
}
