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
    // 1 when the cull pass should record per-thread reject reasons
    // into reject_reasons[] (#454.4). 0 in production; the overlay
    // raster pass flips it on while the user holds a reject-mode
    // dropdown selection.
    debug_active: u32,
    // 1 when the view is orthographic — a shadow cascade. Changes the
    // LOD test rather than tuning it: an orthographic projection
    // magnifies everything equally, so there is no distance term.
    lod_orthographic: u32,
    // Three scalars, NOT `vec3<u32>`: a vec3 aligns to 16, which would
    // push `view_proj` to the next boundary and inflate the struct to
    // 224 bytes against the host's 208. wgpu reports that as
    // "min_binding_size" and it reads like a binding problem.
    _pad_lod0: u32,
    _pad_lod1: u32,
    _pad_lod2: u32,
    // Clip-from-world matrix used by the AABB-vs-frustum test in
    // `atomic.wgsl` (#454.4 follow-up A). The atomic R64 path now
    // matches the Hi-Z 2-pass entry's #488 fix: derive frustum
    // planes from `view_proj * inst.transform` and test the local
    // AABB instead of the bounding sphere — closes the silhouette
    // holes sphere-bounds left at viewport edges. Legacy entries
    // (basic / scene / pool) ignore the field; their pipelines stay
    // on the pre-extracted `planes[]`.
    view_proj: mat4x4<f32>,
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

// Largest axis scale of an instance's transform.
//
// 🔴 A meshlet's simplification error is measured in the mesh's OWN
// space. Scale the instance up and the same error covers more world,
// and more of the shadow map with it — so the selector has to scale it
// too or a large object is judged by a small object's budget, keeps a
// LOD coarser than it should, and self-shadows against a silhouette
// that no longer matches the one being shaded. Blotches on the object,
// only on the ones somebody scaled.
//
// Bevy applies exactly this in `lod_error_is_imperceptible`
// (`simplification_error * world_scale`).
fn instance_world_scale(transform: mat4x4<f32>) -> f32 {
    return max(
        length(transform[0].xyz),
        max(length(transform[1].xyz), length(transform[2].xyz)),
    );
}

fn camera_in_cone(apex: vec3<f32>, axis: vec3<f32>, cutoff: f32) -> bool {
    // 🔴 Never for an orthographic view — a shadow cascade.
    //
    // The whole test is "form the vector from the viewpoint to the
    // meshlet and compare it against the average normal", and a
    // directional light has no viewpoint. Any point stood in for one is
    // an approximation that gets worse the further the meshlet is from
    // it, and it fails in the direction that matters: meshlets facing
    // the sun get rejected, write no depth, and the shadow comes out
    // with bites taken out of its edge. It shows up most when the caster
    // is off screen, because that is when the stand-in point is
    // furthest from the geometry.
    //
    // Bevy 0.19 does not cone-cull meshlets in ANY view — there is no
    // occurrence of "cone" in `meshlet_cull_shared.wgsl`. Keeping it for
    // the camera, where the viewpoint is real, and dropping it here.
    if (params.lod_orthographic == 1u) {
        return false;
    }
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
    // Reversed-Z (#488): closer to cam = larger ndc.z. The legacy
    // pyramid uses `cs_reduce_max` so its tile value under reversed
    // depth = the CLOSEST fragment in the tile. Conservative test:
    // sphere's NEAREST ndc.z (= centre + radius extent) is BEHIND
    // (smaller than) the tile's nearest → occluded.
    let sphere_nearest_depth = ndc.z + 2.0 * radius / clip.w;
    return sphere_nearest_depth < max_depth;
}

// Naga's pipeline-layout validation walks the call graph of each entry
// point and demands a binding slot for every global the function tree
// reaches. Splitting the cull body into two siblings (one with the
// Hi-Z branch, one without) is the cleanest way to make `cs_cull`'s
// pipeline layout `[group(0)]` and `cs_cull_hi_z`'s layout
// `[group(0), group(1)]` — naga sees the Hi-Z bindings only from the
// second branch, never references them from `cs_cull`.

// A dispatch dimension holds at most 65 535 workgroups — 4 194 240
// threads at 64 per group. A scene-wide cull runs one thread per
// (instance × meshlet), so an open world reaches that with a few
// hundred detailed models and the dispatch is rejected outright.
//
// The host folds the excess into a second dimension
// (`kooch_core::gpu::tiled_workgroups`); this reverses it. Below the
// ceiling `gid.y` is 0 and this is exactly `gid.x`, so the 1-D path
// costs one multiply-add and nothing else.
//
// ⚠️ The tiled form OVER-COVERS: the last row dispatches threads past
// the real count. Every `run_*` below guards on its own total, and
// must keep doing so.
fn linear_thread(gid: vec3<u32>, groups: vec3<u32>) -> u32 {
    return gid.y * (groups.x * 64u) + gid.x;
}
