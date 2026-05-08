
// ---------------------------------------------------------------
// Hi-Z 2-pass scene-pool cull (#445).
//
// Pass A (`cs_cull_scene_pool_atomic_hi_z`) mirrors
// `cs_cull_scene_pool_atomic` exactly but adds a Hi-Z test against
// the *previous frame's* pyramid (`hi_z_pyramid_atomic`) at the very
// tail. Meshlets that survive frustum + cone but fail Hi-Z are
// appended to `culled_meshlets[]`; pass B (lands in T3) re-tests
// them against this frame's freshly-built pyramid so anything that
// became visible since the previous frame slips back in.
//
// New bindings (consumed only by this entry + pass B):
//   group(0) @ binding(4): culled_meshlets — append target for
//     Hi-Z rejects, packed identically to visible_meshlets.
//   group(0) @ binding(5): culled_count — atomic counter for the
//     above; pass B reads it as a workgroup count.
//   group(2) @ binding(2): hi_z_params_atomic — view_proj +
//     pyramid dimensions for the previous-frame pyramid sample.
//   group(2) @ binding(3): hi_z_pyramid_atomic — multi-mip R32Float
//     view of the previous-frame pyramid.
//
// Existing entry points keep their old layout (cull_bgl / scene_bgl
// unchanged); the Hi-Z entry uses extended layouts the dispatcher
// builds separately so the shader file can host both shapes.
// ---------------------------------------------------------------

@group(0) @binding(4) var<storage, read_write> culled_meshlets: array<u32>;
@group(0) @binding(5) var<storage, read_write> culled_count: atomic<u32>;
@group(2) @binding(2) var<uniform> hi_z_params_atomic: HiZParams;
@group(2) @binding(3) var hi_z_pyramid_atomic: texture_2d<f32>;

// AABB-based occlusion test ported from Bevy's
// `crates/bevy_pbr/src/meshlet/meshlet_cull_shared.wgsl`
// (`should_occlusion_cull_aabb` family). Sphere-bounds + small-angle
// approximation produced silhouette holes on close-up models in
// PR #487 that no amount of conservative-shift tuning could close
// cleanly; AABB 8-corner projection (zeux's algorithm,
// https://zeux.io/2023/01/12/approximate-projected-bounds/) plus a
// 16-tap min sample is the standard fix.
//
// Reversed-Z (#488): the depth pyramid stores the FARTHEST fragment
// per tile as the SMALLEST ndc.z value (because near=1, far=0). The
// conservative occlusion test becomes "is the closest point of the
// AABB (= max ndc.z in reversed-Z) BEHIND the farthest tile fragment
// (= tile min)?" → `aabb.max.z <= tile_min`.

struct ScreenAabb {
    min: vec3<f32>,
    max: vec3<f32>,
}

fn min8_atomic(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>, e: vec3<f32>, f: vec3<f32>, g: vec3<f32>, h: vec3<f32>) -> vec3<f32> {
    return min(min(min(a, b), min(c, d)), min(min(e, f), min(g, h)));
}

fn max8_atomic(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>, e: vec3<f32>, f: vec3<f32>, g: vec3<f32>, h: vec3<f32>) -> vec3<f32> {
    return max(max(max(a, b), max(c, d)), max(max(e, f), max(g, h)));
}

fn min8_4_atomic(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, e: vec4<f32>, f: vec4<f32>, g: vec4<f32>, h: vec4<f32>) -> vec4<f32> {
    return min(min(min(a, b), min(c, d)), min(min(e, f), min(g, h)));
}

// AABB-vs-frustum (positive-vertex test). Ports Bevy's
// `aabb_in_frustum` from meshlet_cull_shared.wgsl. Uses 5 planes
// only (4 lateral + ndc.z >= 0); the second z-plane is dropped
// intentionally so meshlets straddling near don't get rejected
// by the cull — the rasterizer clips them against near anyway,
// and rejecting here causes silhouette holes at viewport edges
// where projected AABBs partially leave the frustum (#488 follow-up).
//
// Planes are extracted GPU-side from `clip_from_local`. The
// `transpose` puts row-i of the matrix into `row_major[i]`, then
// Gribb-Hartmann gives the 6 standard planes; we keep only 5.
//
// `flipped = half_extent * sign(plane.xyz)` is the offset from
// AABB centre to its "positive vertex" w.r.t. the plane normal.
// If the positive vertex is outside the half-space, the entire
// AABB is outside.
//
// Returns true iff AABB is outside the frustum (= reject).
fn aabb_outside_frustum_atomic(
    world_from_local: mat4x4<f32>,
    aabb_min_local: vec3<f32>,
    aabb_max_local: vec3<f32>,
) -> bool {
    let center = (aabb_min_local + aabb_max_local) * 0.5;
    let half_extent = (aabb_max_local - aabb_min_local) * 0.5;
    let clip_from_local = hi_z_params_atomic.view_proj * world_from_local;
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

// Projects an AABB's 8 corners to clip space, divides by w, and
// returns the screen-space [0,1] rectangle + min/max NDC depth.
// Returns `false` when the camera lies INSIDE the AABB (one or more
// corners cross the near plane), in which case occlusion culling
// must NOT happen — the perspective divide would flip signs.
fn project_aabb_atomic(
    clip_from_local: mat4x4<f32>,
    near: f32,
    aabb_min_local: vec3<f32>,
    aabb_max_local: vec3<f32>,
    out: ptr<function, ScreenAabb>,
) -> bool {
    let extent = aabb_max_local - aabb_min_local;
    let sx = clip_from_local * vec4<f32>(extent.x, 0.0, 0.0, 0.0);
    let sy = clip_from_local * vec4<f32>(0.0, extent.y, 0.0, 0.0);
    let sz = clip_from_local * vec4<f32>(0.0, 0.0, extent.z, 0.0);

    let p0 = clip_from_local * vec4<f32>(aabb_min_local, 1.0);
    let p1 = p0 + sz;
    let p2 = p0 + sy;
    let p3 = p2 + sz;
    let p4 = p0 + sx;
    let p5 = p4 + sz;
    let p6 = p4 + sy;
    let p7 = p6 + sz;

    let depth = min8_4_atomic(p0, p1, p2, p3, p4, p5, p6, p7).w;
    if (depth < near) {
        return false;
    }

    let dp0 = p0.xyz / p0.w;
    let dp1 = p1.xyz / p1.w;
    let dp2 = p2.xyz / p2.w;
    let dp3 = p3.xyz / p3.w;
    let dp4 = p4.xyz / p4.w;
    let dp5 = p5.xyz / p5.w;
    let dp6 = p6.xyz / p6.w;
    let dp7 = p7.xyz / p7.w;
    let mn = min8_atomic(dp0, dp1, dp2, dp3, dp4, dp5, dp6, dp7);
    let mx = max8_atomic(dp0, dp1, dp2, dp3, dp4, dp5, dp6, dp7);
    var vaabb = vec4<f32>(mn.xy, mx.xy);
    // ndc → texture UV: rescale to [0, 1] and flip Y.
    vaabb = vaabb.xwzy * vec4<f32>(0.5, -0.5, 0.5, -0.5) + 0.5;
    (*out).min = vec3<f32>(vaabb.xy, mn.z);
    (*out).max = vec3<f32>(vaabb.zw, mx.z);
    return true;
}

fn sample_hzb_row_atomic(sx: vec4<u32>, sy: u32, mip: i32) -> f32 {
    let a = textureLoad(hi_z_pyramid_atomic, vec2(sx.x, sy), mip).x;
    let b = textureLoad(hi_z_pyramid_atomic, vec2(sx.y, sy), mip).x;
    let c = textureLoad(hi_z_pyramid_atomic, vec2(sx.z, sy), mip).x;
    let d = textureLoad(hi_z_pyramid_atomic, vec2(sx.w, sy), mip).x;
    return min(min(a, b), min(c, d));
}

fn sample_hzb_atomic(smin: vec2<u32>, smax: vec2<u32>, mip: i32) -> f32 {
    let texel = vec4<u32>(0u, 1u, 2u, 3u);
    let sx = min(smin.x + texel, smax.xxxx);
    let sy = min(smin.y + texel, smax.yyyy);
    // 4×4 = 16-tap: covers the AABB's screen footprint at the chosen
    // mip with conservative neighbour overlap.
    let a = sample_hzb_row_atomic(sx, sy.x, mip);
    let b = sample_hzb_row_atomic(sx, sy.y, mip);
    let c = sample_hzb_row_atomic(sx, sy.z, mip);
    let d = sample_hzb_row_atomic(sx, sy.w, mip);
    return min(min(a, b), min(c, d));
}

fn occlusion_cull_screen_aabb_atomic(aabb: ScreenAabb) -> bool {
    let hzb_size = hi_z_params_atomic.hi_z_size;
    let aabb_min = aabb.min.xy * hzb_size;
    let aabb_max = aabb.max.xy * hzb_size;

    let min_texel = vec2<u32>(max(aabb_min, vec2<f32>(0.0)));
    let max_texel = vec2<u32>(min(aabb_max, hzb_size - 1.0));
    let size = max_texel - min_texel;
    let max_size = max(size.x, size.y);

    // firstLeadingBit(0) wraps to ~0u; +1 then -2u rolls the
    // overflow into the small-mip case the 4×4 tap already covers.
    var mip = max(firstLeadingBit(max_size) + 1u, 2u) - 2u;
    if (any((max_texel >> vec2(mip)) > (min_texel >> vec2(mip)) + 3u)) {
        mip += 1u;
    }
    mip = min(mip, hi_z_params_atomic.hi_z_mip_count - 1u);

    let smin = min_texel >> vec2<u32>(mip);
    let smax = max_texel >> vec2<u32>(mip);

    let curr_depth = sample_hzb_atomic(smin, smax, i32(mip));
    // Reversed-Z conservative test: closest point of AABB (max ndc.z)
    // is BEHIND tile's farthest fragment (min sample) → occluded.
    return aabb.max.z <= curr_depth;
}

// AABB-based occlusion entry point used by pass A + pass B in the
// scene-pool 2-pass cull. `world_from_local = inst.transform`,
// `clip_from_world = hi_z_params_atomic.view_proj`.
fn occluded_by_hi_z_atomic(
    world_from_local: mat4x4<f32>,
    aabb_min_local: vec3<f32>,
    aabb_max_local: vec3<f32>,
) -> bool {
    let projection = hi_z_params_atomic.view_proj;
    // Bevy's near-plane extraction handles ortho vs perspective; under
    // reversed-Z perspective the reading is still proj[3][2].
    var near: f32;
    if (projection[3][3] == 1.0) {
        near = projection[3][2] / projection[2][2];
    } else {
        near = projection[3][2];
    }

    let clip_from_local = projection * world_from_local;
    var screen_aabb = ScreenAabb(vec3<f32>(0.0), vec3<f32>(0.0));
    if (project_aabb_atomic(clip_from_local, near, aabb_min_local, aabb_max_local, &screen_aabb)) {
        return occlusion_cull_screen_aabb_atomic(screen_aabb);
    }
    return false;
}

fn run_cull_scene_pool_atomic_hi_z(thread_id: u32) {
    // Mirror of run_cull_scene_pool_atomic with a Hi-Z test injected
    // before the visible-emit. Keep these two functions in lock-step
    // when LOD / frustum / cone logic changes — there is no shared
    // helper because the only difference is the tail decision and a
    // shared helper would have to take every binding by parameter.
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

    if (inst.lod_force_level >= 0) {
        if (i32(m.lod_level) != inst.lod_force_level) {
            return;
        }
    } else if (params.debug_mode == 8u) {
        if (m.lod_error != 0.0) {
            return;
        }
    } else if (params.debug_mode == 9u) {
        if (m.parent_meshlet_index != 0xFFFFFFFFu) {
            return;
        }
    } else {
        let target_px = params.lod_target_error_pixels;

        var above_too_coarse: bool;
        if (m.group_index == 0xFFFFFFFFu) {
            above_too_coarse = true;
        } else {
            let local_group = m.group_index - mesh_desc.group_base;
            let slot = inst.group_base + local_group;
            let bits = atomicLoad(&group_max_err[slot]);
            let group_err_px = bitcast<f32>(bits);
            above_too_coarse = group_err_px > target_px;
        }

        var below_fine: bool;
        if (m.children_group_index == 0xFFFFFFFFu) {
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

    // AABB-vs-frustum (Bevy parity). Sphere bounds were rejecting
    // meshlets at viewport edges whose AABB still overlapped the
    // frustum — caused silhouette holes on close-up models. Drops
    // far plane on purpose; rasterizer clips at near.
    if (aabb_outside_frustum_atomic(inst.transform, m.aabb_min, m.aabb_max)) {
        return;
    }

    let world_apex = (inst.transform * vec4<f32>(m.cone_apex, 1.0)).xyz;
    let world_axis = normalize(
        (inst.transform * vec4<f32>(m.cone_axis, 0.0)).xyz
    );
    if (camera_in_cone(world_apex, world_axis, m.cone_cutoff)) {
        return;
    }

    let packed = (instance_id << 16u) | (global_meshlet_idx & 0xffffu);

    if (occluded_by_hi_z_atomic(inst.transform, m.aabb_min, m.aabb_max)) {
        // Hi-Z reject: defer to pass B with this frame's pyramid.
        let slot = atomicAdd(&culled_count, 1u);
        culled_meshlets[slot] = packed;
        return;
    }

    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = packed;
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_scene_pool_atomic_hi_z(@builtin(global_invocation_id) gid: vec3<u32>) {
    run_cull_scene_pool_atomic_hi_z(gid.x);
}

// ---------------------------------------------------------------
// Pass B (#445).
//
// Iterates the compact `culled_meshlets[]` array pass A populated
// and re-tests every entry against the *current* frame's Hi-Z
// pyramid (orchestrator binds `hi_z_pyramid_atomic` to `hiz_curr`
// for this dispatch). Survivors append to `visible_meshlets`.
//
// Frustum / cone / LOD are NOT re-checked — pass A already cleared
// those; the only reason a meshlet landed in `culled_meshlets` is
// the previous frame's Hi-Z said it was occluded, which can be a
// false negative if geometry moved or rotated into view.
//
// Dispatch shape: workgroup count = `ceil(capacity / 64)` — the
// worst case where every meshlet was occluded. Threads past
// `culled_count` early-out, paying only an atomic load. This
// avoids a CPU readback of culled_count + a separate indirect
// dispatch buffer; with `wgpu::DispatchIndirect` we could trim the
// dispatch tighter, but the early-out cost is ~one atomic load per
// surplus thread and the readback would be a CPU stall.
// ---------------------------------------------------------------

fn run_cull_pass_b(thread_id: u32) {
    let count = atomicLoad(&culled_count);
    if (thread_id >= count) {
        return;
    }
    let packed = culled_meshlets[thread_id];
    let instance_id = packed >> 16u;
    let global_meshlet_idx = packed & 0xffffu;

    let inst = instances[instance_id];
    let m = pool_meshlets[global_meshlet_idx];

    let world_center = (inst.transform * vec4<f32>(m.bounds_center, 1.0)).xyz;
    if (occluded_by_hi_z_atomic(inst.transform, m.aabb_min, m.aabb_max)) {
        // Still occluded against this frame's pyramid → drop.
        return;
    }

    let slot = atomicAdd(&visible_count, 1u);
    visible_meshlets[slot] = packed;
}

@compute @workgroup_size(64, 1, 1)
fn cs_cull_pass_b(@builtin(global_invocation_id) gid: vec3<u32>) {
    run_cull_pass_b(gid.x);
}
