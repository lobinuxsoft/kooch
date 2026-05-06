// meshlet_cull.wgsl — frustum + backface cone + (optional) Hi-Z cull.
//
// One thread per meshlet. Each thread reads its descriptor and runs:
//   1. frustum: bounding sphere vs the camera's six planes.
//   2. backface cone: dot(normalize(cone_apex - camera), cone_axis) >= cone_cutoff
//      → the camera lies on the meshlet's back-facing side; cull.
//      `cone_axis` follows meshopt's convention (points along the
//      front-face normals). The test direction is camera→apex, matching
//      Bevy's `view_to_meshlet` formulation.
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
    // Continuous-LOD selector (#442). A meshlet survives selection when
    //   my_pixel_error <= lod_target_error_pixels &&
    //   parent_pixel_error > lod_target_error_pixels
    // Roots (parent == 0xFFFFFFFFu) always pass.
    lod_target_error_pixels: f32,
    // Precomputed `0.5 * viewport_height_px * proj_scale_y` so each
    // shader thread recovers the pixel error as
    // `lod_error / distance * factor`. Set to 0 to disable the
    // selector — every root meshlet then passes (legacy single-LOD
    // behaviour) and non-roots are rejected because their parent's
    // pixel error collapses to 0 ≤ threshold.
    lod_error_to_pixel_factor: f32,
    // Mirrors MeshletDebugMode discriminant. Value 8 (OnlyLod0) and
    // 9 (OnlyRoots) override the LOD selector — see
    // cs_cull_scene_pool_atomic.
    debug_mode: u32,
    _pad3: u32,
}

struct MeshletDescriptor {
    vertex_offset: u32,
    triangle_offset: u32,
    vertex_count: u32,
    triangle_count: u32,
    aabb_min: vec3<f32>,
    // DAG: parent meshlet index (#442). u32::MAX sentinel = root.
    parent_meshlet_index: u32,
    aabb_max: vec3<f32>,
    // DAG: meshopt::simplify error this meshlet represents.
    lod_error: f32,
    bounds_center: vec3<f32>,
    bounding_radius: f32,
    cone_apex: vec3<f32>,
    cone_cutoff: f32,
    cone_axis: vec3<f32>,
    // #465 group ids — atomicMax pixel error per group in pass 1, then
    // pass 2 reads group_max_err[group_index] for "above_too_coarse"
    // and group_max_err[children_group_index] for "below_fine".
    // 0xFFFFFFFFu sentinel = no group on that side (root or LOD 0).
    group_index: u32,
    children_group_index: u32,
    // Chain depth for LOD-stack inspector + per-instance level lock
    // (#467). 0 = LOD 0, increments per simplification step.
    lod_level: u32,
    _pad4: u32,
    _pad5: u32,
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

fn camera_in_cone(apex: vec3<f32>, axis: vec3<f32>, cutoff: f32) -> bool {
    // meshopt sets cone_cutoff to 1.0 when the meshlet's normals are
    // too divergent for a meaningful cone. Treat that as a no-cull
    // sentinel — the test would otherwise reject everything.
    if (cutoff >= 1.0) {
        return false;
    }
    // `axis` follows meshopt's convention: it points along the
    // average front-face normal of the meshlet. Bevy/UE5-style
    // backface test: form the camera-to-apex vector and compare its
    // alignment with the axis. When the camera sits on the back-facing
    // half-space, `camera_to_apex` aligns with `axis` (both point
    // outwards from the meshlet's surface relative to the camera) and
    // `dot >= cutoff` triggers the cull. When the camera is in front
    // the two vectors are opposed, the dot product is negative, and
    // the meshlet renders.
    let to_apex = apex - params.camera_position;
    let len_sq = dot(to_apex, to_apex);
    if (len_sq == 0.0) {
        return false;
    }
    let view = to_apex / sqrt(len_sq);
    return dot(view, axis) >= cutoff;
}

fn camera_in_backface_cone(desc: MeshletDescriptor) -> bool {
    return camera_in_cone(desc.cone_apex, desc.cone_axis, desc.cone_cutoff);
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
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

struct SceneCullParams {
    instance_count: u32,
    meshlets_per_mesh: u32,
    _pad0: u32,
    _pad1: u32,
}

@group(2) @binding(0) var<storage, read> instances: array<MeshInstance>;
@group(2) @binding(1) var<uniform> scene_params: SceneCullParams;

// Projects a meshlet's world-space LOD error to pixels at the camera.
// Standard perspective formula: `error * scale_y * viewport_h / (2 * dist)`,
// where `scale_y = 1 / tan(fovy/2)`. The CPU side rolls
// `0.5 * viewport_h * scale_y` into `lod_error_to_pixel_factor`.
fn lod_pixel_error(lod_error: f32, world_center: vec3<f32>) -> f32 {
    let to_cam = world_center - params.camera_position;
    let dist = max(length(to_cam), 0.0001);
    return lod_error * params.lod_error_to_pixel_factor / dist;
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
        let my_err_px = lod_pixel_error(desc.lod_error, world_center_self);
        let parent_err_px = lod_pixel_error(parent.lod_error, world_center_parent);
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
fn cs_cull_scene(@builtin(global_invocation_id) gid: vec3<u32>) {
    run_cull_scene(gid.x);
}

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

fn lod_pixel_error_pool(lod_error: f32, world_center: vec3<f32>) -> f32 {
    let to_cam = world_center - params.camera_position;
    let dist = max(length(to_cam), 0.0001);
    return lod_error * params.lod_error_to_pixel_factor / dist;
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
        let my_err_px = lod_pixel_error_pool(desc.lod_error, world_center_self);
        let parent_err_px = lod_pixel_error_pool(parent.lod_error, world_center_parent);
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
