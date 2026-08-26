// surface_reconstruct.wgsl — visibility-buffer → surface attributes.
//
// Given a decoded `(visible_slot, tri_idx)` and a pixel, rebuilds the
// triangle and interpolates world position / normal / uv plus analytical
// uv derivatives (for correct mip selection) through perspective-correct
// barycentrics.
//
// WGSL has no #include, so this file is CONCATENATED in Rust ahead of
// each consumer. It declares the geometry bindings on groups 1 and 3 —
// identical across both shading paths — and expects the consumer to
// declare `camera` (with `view_proj`) and `screen` (with `size`), which
// both already do.
//
// # Why both paths share this
//
// The R64 two-pass path had barycentric reconstruction; the R32 compute
// path averaged the triangle's three vertex normals and had no world
// position at all. That was invisible while shading was `normal × 0.5 +
// 0.5`, and stops being invisible the moment a point light needs to know
// how far away the surface is: the fallback path would have lit the
// centroid of every triangle. What differs between the paths is how the
// visibility buffer is *read* — 32-bit texture vs 64-bit storage — so
// that is the only part each one keeps.
//
// compute_partial_derivatives is a near-verbatim port of Bevy's
// visibility_buffer_resolve.wgsl, itself derived from The-Forge's
// Visibility-Buffer analytical derivative maths (vb_shading_utilities).

struct MeshVertexStored {
    position: array<f32, 3>,
    normal: array<f32, 3>,
    uv: array<f32, 2>,
}

struct MeshletDescriptor {
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

// Must mirror the cull-side struct (stride 96 B) so `instances[i]` reads
// from the correct byte offset (#474 grew this from 80 → 96 B).
struct MeshInstance {
    transform: mat4x4<f32>,
    mesh_id: u32,
    material_id: u32,
    lod_bias: f32,
    lod_force_level: i32,
    group_base: u32,
    // #804 — per-instance bits; bit 0 is "receives shadows". Was
    // `_pad0`, so the 96-byte stride is unchanged.
    flags: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(1) @binding(0) var<storage, read> vertices: array<MeshVertexStored>;
@group(1) @binding(1) var<storage, read> meshlet_vertices: array<u32>;
@group(1) @binding(2) var<storage, read> meshlet_triangles: array<u32>;
@group(1) @binding(3) var<storage, read> descriptors: array<MeshletDescriptor>;

@group(3) @binding(0) var<storage, read> visible_meshlets: array<u32>;
@group(3) @binding(1) var<storage, read> instances: array<MeshInstance>;

/// Reconstructed per-fragment surface attributes.
struct VertexOutput {
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    uv: vec2<f32>,
    ddx_uv: vec2<f32>,
    ddy_uv: vec2<f32>,
    world_tangent: vec4<f32>,
    material_id: u32,
    // #804 — the instance's bits, carried through so the shading path
    // can skip a shadow fetch this surface never wanted.
    flags: u32,
}

struct PartialDerivatives {
    barycentrics: vec3<f32>,
    ddx: vec3<f32>,
    ddy: vec3<f32>,
}

fn fetch_local_vertex_index(byte_offset: u32) -> u32 {
    let word_idx = byte_offset / 4u;
    let byte_in_word = byte_offset & 3u;
    let packed = meshlet_triangles[word_idx];
    return (packed >> (byte_in_word * 8u)) & 0xffu;
}

fn position_world_to_clip(world: vec3<f32>) -> vec4<f32> {
    return camera.view_proj * vec4<f32>(world, 1.0);
}

// Pixel-space frag coord → NDC xy in [-1, 1], y up.
fn frag_coord_to_ndc(frag_coord: vec2<f32>) -> vec2<f32> {
    let uv = frag_coord / vec2<f32>(screen.size);
    return vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
}

// Perspective-correct barycentrics + analytical screen-space derivatives.
// Verbatim structure from Bevy/The-Forge; layout-agnostic.
// 🔴 `two_over_screen_size`, and the name is the whole story. NDC spans
// 2 units across `screen_size` pixels, so converting a derivative from
// "per NDC unit" to "per pixel" multiplies by `2 / screen_size`. The
// upstream this is ported from names the parameter exactly that —
// The-Forge's `CalcFullBary(..., float2 two_over_windowsize)` — and
// Bevy renamed it to `half_screen_size` and passes `viewport.zw / 2.0`,
// which is its RECIPROCAL. We ported the name and the arithmetic with
// it.
//
// It is not merely a scale. The value feeds `1 / (interp_inv_w +
// ddx_sum)` below: at the right magnitude `ddx_sum` is negligible and
// what comes out is the derivative; at 250000x it dominates the divide
// and the expression collapses onto `-barycentrics`, which is a
// position inside the triangle and has no relationship to the camera at
// all. Measured before the fix: mip level 10 at two metres and level
// 0.6 at forty.
fn compute_partial_derivatives(
    world_positions: array<vec4<f32>, 3>,
    ndc_uv: vec2<f32>,
    two_over_screen_size: vec2<f32>,
) -> PartialDerivatives {
    var result: PartialDerivatives;

    let clip_0 = position_world_to_clip(world_positions[0].xyz);
    let clip_1 = position_world_to_clip(world_positions[1].xyz);
    let clip_2 = position_world_to_clip(world_positions[2].xyz);

    let inv_w = 1.0 / vec3<f32>(clip_0.w, clip_1.w, clip_2.w);
    let ndc_0 = clip_0.xy * inv_w[0];
    let ndc_1 = clip_1.xy * inv_w[1];
    let ndc_2 = clip_2.xy * inv_w[2];

    let inv_det = 1.0 / determinant(mat2x2<f32>(ndc_2 - ndc_1, ndc_0 - ndc_1));
    result.ddx = vec3<f32>(ndc_1.y - ndc_2.y, ndc_2.y - ndc_0.y, ndc_0.y - ndc_1.y) * inv_det * inv_w;
    result.ddy = vec3<f32>(ndc_2.x - ndc_1.x, ndc_0.x - ndc_2.x, ndc_1.x - ndc_0.x) * inv_det * inv_w;

    var ddx_sum = dot(result.ddx, vec3<f32>(1.0));
    var ddy_sum = dot(result.ddy, vec3<f32>(1.0));

    let delta_v = ndc_uv - ndc_0;
    let interp_inv_w = inv_w.x + delta_v.x * ddx_sum + delta_v.y * ddy_sum;
    let interp_w = 1.0 / interp_inv_w;

    result.barycentrics = vec3<f32>(
        interp_w * (inv_w[0] + delta_v.x * result.ddx.x + delta_v.y * result.ddy.x),
        interp_w * (delta_v.x * result.ddx.y + delta_v.y * result.ddy.y),
        interp_w * (delta_v.x * result.ddx.z + delta_v.y * result.ddy.z),
    );

    result.ddx *= two_over_screen_size.x;
    result.ddy *= two_over_screen_size.y;
    ddx_sum *= two_over_screen_size.x;
    ddy_sum *= two_over_screen_size.y;

    result.ddy *= -1.0;
    ddy_sum *= -1.0;

    let interp_ddx_w = 1.0 / (interp_inv_w + ddx_sum);
    let interp_ddy_w = 1.0 / (interp_inv_w + ddy_sum);

    result.ddx = interp_ddx_w * (result.barycentrics * interp_inv_w + result.ddx) - result.barycentrics;
    result.ddy = interp_ddy_w * (result.barycentrics * interp_inv_w + result.ddy) - result.barycentrics;
    return result;
}

// Screen-space tangent from world-position + uv derivatives (Lengyel).
// w carries handedness for the bitangent reconstruction in the shader.
fn calculate_world_tangent(
    world_normal: vec3<f32>,
    ddx_world_position: vec3<f32>,
    ddy_world_position: vec3<f32>,
    ddx_uv: vec2<f32>,
    ddy_uv: vec2<f32>,
) -> vec4<f32> {
    let denom = ddx_uv.x * ddy_uv.y - ddy_uv.x * ddx_uv.y;
    // Degenerate uv gradient (flat-shaded / no map): fall back to an
    // arbitrary tangent orthogonal to the normal.
    if (abs(denom) < 1e-8) {
        let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(world_normal.z) > 0.999);
        let t = normalize(cross(up, world_normal));
        return vec4<f32>(t, 1.0);
    }
    let r = 1.0 / denom;
    let tangent = (ddx_world_position * ddy_uv.y - ddy_world_position * ddx_uv.y) * r;
    let bitangent = (ddy_world_position * ddx_uv.x - ddx_world_position * ddy_uv.x) * r;
    // Gram-Schmidt orthonormalize against the interpolated normal.
    let t = normalize(tangent - world_normal * dot(world_normal, tangent));
    let w = select(-1.0, 1.0, dot(cross(world_normal, t), bitangent) > 0.0);
    return vec4<f32>(t, w);
}

fn corner_world_position(inst: MeshInstance, global_vertex: u32) -> vec4<f32> {
    let v = vertices[global_vertex];
    let local = vec4<f32>(v.position[0], v.position[1], v.position[2], 1.0);
    return inst.transform * local;
}

fn corner_world_normal(inst: MeshInstance, global_vertex: u32) -> vec3<f32> {
    let v = vertices[global_vertex];
    let local = vec3<f32>(v.normal[0], v.normal[1], v.normal[2]);
    return (inst.transform * vec4<f32>(local, 0.0)).xyz;
}

fn corner_uv(global_vertex: u32) -> vec2<f32> {
    let v = vertices[global_vertex];
    return vec2<f32>(v.uv[0], v.uv[1]);
}

fn global_vertex_id(desc: MeshletDescriptor, tri_idx: u32, corner: u32) -> u32 {
    let byte_offset = desc.triangle_offset + tri_idx * 3u + corner;
    let local = fetch_local_vertex_index(byte_offset);
    return meshlet_vertices[desc.vertex_offset + local];
}

/// Full attribute reconstruction for an already-decoded visibility
/// sample. `frag_coord` is in pixels.
///
/// Each path decodes `(visible_slot, tri_idx)` from its own visibility
/// buffer — the R64 path packs `slot << 7`, the R32 path packs
/// `(slot + 1) << 7` so zero can mean background — so the decode stays
/// with the reader and everything downstream of it is shared.
fn resolve_surface(visible_slot: u32, tri_idx: u32, frag_coord: vec2<f32>) -> VertexOutput {
    let packed_visible = visible_meshlets[visible_slot];
    let inst_id = packed_visible >> 16u;
    let meshlet_id = packed_visible & 0xffffu;

    let inst = instances[inst_id];
    let desc = descriptors[meshlet_id];

    let g0 = global_vertex_id(desc, tri_idx, 0u);
    let g1 = global_vertex_id(desc, tri_idx, 1u);
    let g2 = global_vertex_id(desc, tri_idx, 2u);

    let wp0 = corner_world_position(inst, g0);
    let wp1 = corner_world_position(inst, g1);
    let wp2 = corner_world_position(inst, g2);

    let ndc = frag_coord_to_ndc(frag_coord);
    let two_over_screen = 2.0 / vec2<f32>(screen.size);
    let pd = compute_partial_derivatives(array<vec4<f32>, 3>(wp0, wp1, wp2), ndc, two_over_screen);

    let world_position = (mat3x4<f32>(wp0, wp1, wp2) * pd.barycentrics).xyz;

    let world_positions = mat3x3<f32>(wp0.xyz, wp1.xyz, wp2.xyz);
    let ddx_world_position = world_positions * pd.ddx;
    let ddy_world_position = world_positions * pd.ddy;

    let n0 = corner_world_normal(inst, g0);
    let n1 = corner_world_normal(inst, g1);
    let n2 = corner_world_normal(inst, g2);
    let world_normal = normalize(mat3x3<f32>(n0, n1, n2) * pd.barycentrics);

    let uv0 = corner_uv(g0);
    let uv1 = corner_uv(g1);
    let uv2 = corner_uv(g2);
    let uv = mat3x2<f32>(uv0, uv1, uv2) * pd.barycentrics;
    let ddx_uv = mat3x2<f32>(uv0, uv1, uv2) * pd.ddx;
    let ddy_uv = mat3x2<f32>(uv0, uv1, uv2) * pd.ddy;

    let world_tangent = calculate_world_tangent(
        world_normal,
        ddx_world_position,
        ddy_world_position,
        ddx_uv,
        ddy_uv,
    );

    var out: VertexOutput;
    out.world_position = world_position;
    out.world_normal = world_normal;
    out.uv = uv;
    out.ddx_uv = ddx_uv;
    out.ddy_uv = ddy_uv;
    out.world_tangent = world_tangent;
    out.material_id = inst.material_id;
    out.flags = inst.flags;
    return out;
}
