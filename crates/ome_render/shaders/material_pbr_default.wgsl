// material_pbr_default.wgsl — default per-material shading body (#440).
//
// CONCATENATED after visibility_buffer_resolve.wgsl (WGSL has no
// #include). That chunk provides `resolve_vertex_output`, the `screen`
// / `camera` uniforms, and the geometry bindings on groups 0/1/3. This
// body adds the material storage (group 2) and the per-material texture
// bind group (group 4) produced by MaterialTexturePool.
//
// Pass 2 of the two-pass path: run once per registered material with the
// material-depth target bound read-only and `CompareFunction::Equal`, so
// only this material's pixels survive. `screen.material_id` selects the
// MaterialParams slot for this pass.
//
// Shading here is normal-debug modulated by the sampled albedo — enough
// to prove texture sampling + tangent-space normal mapping end to end.
// Real Cook-Torrance lighting (sun + IBL) lands with #441; metal/rough
// is bound and reserved for it.

struct MaterialParams {
    base_color: vec4<f32>,
    metallic_roughness_emissive_pad: vec4<f32>,
    texture_indices: vec4<u32>,
}

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

@group(4) @binding(0) var albedo_tex: texture_2d<f32>;
@group(4) @binding(1) var normal_tex: texture_2d<f32>;
@group(4) @binding(2) var metal_rough_tex: texture_2d<f32>;
@group(4) @binding(3) var material_sampler: sampler;

struct FsInput {
    @builtin(position) position: vec4<f32>,
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

    // Analytical uv derivatives → correct mip selection. Automatic quad
    // derivatives are wrong here: neighbouring fragments in the same 2×2
    // quad may reconstruct from different triangles.
    let albedo = textureSampleGrad(
        albedo_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv);
    let base = albedo.rgb * mat.base_color.rgb;

    // Perturb the interpolated normal by the tangent-space normal map.
    let n_ts = textureSampleGrad(
        normal_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv).xyz * 2.0 - 1.0;
    let n = normalize(surf.world_normal);
    let t = normalize(surf.world_tangent.xyz);
    let b = cross(n, t) * surf.world_tangent.w;
    let world_n = normalize(mat3x3<f32>(t, b, n) * n_ts);

    let normal_debug = world_n * 0.5 + 0.5;
    return vec4<f32>(normal_debug * base, 1.0);
}
