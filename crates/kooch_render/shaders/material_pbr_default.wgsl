// material_pbr_default.wgsl — default per-material shading body (#440),
// lit by Inti (#441).
//
// CONCATENATED after visibility_buffer_resolve.wgsl and after
// inti_pbr.wgsl (WGSL has no #include). The first provides
// `resolve_vertex_output`, the `screen` / `camera` uniforms and the
// geometry bindings on groups 0/1/3; the second provides the shading
// model on group 5. This body adds the material storage (group 2) and
// the per-material texture bind group (group 4).
//
// Pass 2 of the two-pass path: run once per registered material with
// the material-depth target bound read-only and `CompareFunction::Equal`,
// so only this material's pixels survive. `screen.material_id` selects
// the MaterialParams slot for this pass.

struct MaterialParams {
    base_color: vec4<f32>,
    // x metallic, y roughness, z emissive, w pad.
    metallic_roughness_emissive_pad: vec4<f32>,
    texture_indices: vec4<u32>,
    // xy tiling, zw offset. See `MaterialParams` in `material/mod.rs`:
    // this struct is declared here and in two other shaders, and a test
    // reads all three because a field added to two of them fails
    // silently rather than at compile time.
    uv_scale_offset: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

@group(4) @binding(0) var albedo_tex: texture_2d<f32>;
@group(4) @binding(1) var normal_tex: texture_2d<f32>;
@group(4) @binding(2) var metal_rough_tex: texture_2d<f32>;
@group(4) @binding(3) var material_sampler: sampler;

struct FsInput {
    // @invariant: the Equal depth test against the material-depth target
    // demands bit-identical depth from every per-material draw, so the
    // clip position must not be recomputed differently across draws.
    @builtin(position) @invariant position: vec4<f32>,
}

// Fullscreen triangle cover. Emits this pass's material id as clip-space
// depth (`screen.material_id / 65535`) so the fixed-function `Equal`
// depth test against the material-depth target admits only this
// material's pixels — the per-material cull, in hardware, with early-Z.
@vertex
fn vs_fullscreen(@builtin(vertex_index) vertex_index: u32) -> FsInput {
    var out: FsInput;
    let x = f32((vertex_index & 1u) << 2u) - 1.0;
    let y = f32((vertex_index & 2u) << 1u) - 1.0;
    let z = f32(screen.material_id) / 65535.0;
    out.position = vec4<f32>(x, y, z, 1.0);
    return out;
}

@fragment
fn fs_material(in: FsInput) -> @location(0) vec4<f32> {
    let surf = resolve_vertex_output(in.position);
    let mat = materials[screen.material_id];

    // 🔴 The DERIVATIVES scale with the coordinate, and forgetting
    // that is the trap. `textureSampleGrad` picks the mip from how
    // fast the uv moves between pixels; tiling a texture twenty
    // times makes it move twenty times faster, and handing the
    // untiled derivatives selects a level about four steps too
    // sharp. The result is the aliasing the mip chain exists to
    // remove, on exactly the surfaces that asked for tiling.
    let uv = surf.uv * mat.uv_scale_offset.xy + mat.uv_scale_offset.zw;
    let ddx_uv = surf.ddx_uv * mat.uv_scale_offset.xy;
    let ddy_uv = surf.ddy_uv * mat.uv_scale_offset.xy;

    // Analytical uv derivatives → correct mip selection. Automatic quad
    // derivatives are wrong here: neighbouring fragments in the same 2×2
    // quad may reconstruct from different triangles.
    let albedo = textureSampleGrad(
        albedo_tex, material_sampler, uv, ddx_uv, ddy_uv);
    let base = albedo.rgb * mat.base_color.rgb;

    // Perturb the interpolated normal by the tangent-space normal map.
    let n_ts = textureSampleGrad(
        normal_tex, material_sampler, uv, ddx_uv, ddy_uv).xyz * 2.0 - 1.0;
    let n = normalize(surf.world_normal);
    let t = normalize(surf.world_tangent.xyz);
    let b = cross(n, t) * surf.world_tangent.w;
    let world_n = normalize(mat3x3<f32>(t, b, n) * n_ts);

    // The debug views (#743), and the only place this path mentions
    // them. In a production pipeline `inti_debug_is_view` is the stub's
    // literal `false`, so this whole branch — and every view behind it —
    // is gone before the shader is register-allocated.
    if (inti_debug_is_view(screen.debug_mode)) {
        return vec4<f32>(
            inti_debug_view(screen.debug_mode, surf.world_position, world_n, in.position.xy),
            1.0);
    }

    // glTF packing: green is roughness, blue is metallic. The 1×1
    // fallback is white, so a material with no map multiplies its
    // scalars by 1 and there is no branch.
    let mr = textureSampleGrad(
        metal_rough_tex, material_sampler, uv, ddx_uv, ddy_uv);
    let metallic = mat.metallic_roughness_emissive_pad.x * mr.b;
    let roughness = mat.metallic_roughness_emissive_pad.y * mr.g;

    var radiance = inti_shade(
        surf.world_position, world_n, base, metallic, roughness, in.position.xy, surf.flags);
    // Emissive is radiance the surface produces rather than reflects, so
    // it joins before tonemapping and ignores every light in the scene.
    radiance += base * mat.metallic_roughness_emissive_pad.z;

    return vec4<f32>(inti_tonemap(radiance), 1.0);
}
