// kind: surface
// The engine's PBR surface: albedo, tangent-space normal map, packed metal/roughness.

fn surface(input: SurfaceInput) -> SurfaceOutput {
    let mat = materials[input.material_id];

    // 🔴 The DERIVATIVES scale with the coordinate, and forgetting that is the trap.
    let uv = input.uv * mat.uv_scale_offset.xy + mat.uv_scale_offset.zw;
    // A bias is `lod += b`, and `lod` is `log2(footprint)`, so scaling the footprint by `exp2(b)`
    // is the bias exactly (#881).
    let derivative_scale = mat.uv_scale_offset.xy * input.mip_bias_scale;
    let ddx_uv = input.ddx_uv * derivative_scale;
    let ddy_uv = input.ddy_uv * derivative_scale;

    let albedo = textureSampleGrad(albedo_tex, material_sampler, uv, ddx_uv, ddy_uv);
    let base = albedo.rgb * mat.base_color.rgb;

    let n_ts = textureSampleGrad(normal_tex, material_sampler, uv, ddx_uv, ddy_uv).xyz * 2.0 - 1.0;
    let n = normalize(input.world_normal);
    let t = normalize(input.world_tangent.xyz);
    let b = cross(n, t) * input.world_tangent.w;

    // glTF packing: green is roughness, blue is metallic. The 1×1 fallback is white, so a
    // material with no map multiplies its scalars by 1 and there is no branch.
    let mr = textureSampleGrad(metal_rough_tex, material_sampler, uv, ddx_uv, ddy_uv);

    var out: SurfaceOutput;
    out.base_color = base;
    out.normal = normalize(mat3x3<f32>(t, b, n) * n_ts);
    out.metallic = mat.metallic_roughness_emissive_pad.x * mr.b;
    out.roughness = mat.metallic_roughness_emissive_pad.y * mr.g;
    out.emissive = base * mat.metallic_roughness_emissive_pad.z;
    return out;
}
