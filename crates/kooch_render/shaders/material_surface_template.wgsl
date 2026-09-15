// kind: surface
// param base_color: color = (1, 1, 1, 1)
// param albedo: texture = white
// param normal_map: texture = normal
// param metal_roughness: texture = white
// param metallic: float = 0 range(0, 1)
// param roughness: float = 0.5 range(0, 1)
// param emissive: float = 0 range(0, 10)
// param uv_scale: vec2 = (1, 1)
// param uv_offset: vec2 = (0, 0)

// A PBR surface written against its own parameters: edit anything below, or add `// param` lines.
fn surface(input: SurfaceInput) -> SurfaceOutput {
    let p = surface_params(input.material_id);
    let uv = input.uv * p.uv_scale + p.uv_offset;

    let base = sample_albedo(input, uv, p.uv_scale).rgb * p.base_color.rgb;
    let n_ts = sample_normal_map(input, uv, p.uv_scale).xyz * 2.0 - 1.0;
    let n = normalize(input.world_normal);
    let t = normalize(input.world_tangent.xyz);
    let b = cross(n, t) * input.world_tangent.w;
    // glTF packing: green is roughness, blue is metallic.
    let mr = sample_metal_roughness(input, uv, p.uv_scale);

    var out: SurfaceOutput;
    out.base_color = base;
    out.normal = normalize(mat3x3<f32>(t, b, n) * n_ts);
    out.metallic = p.metallic * mr.b;
    out.roughness = p.roughness * mr.g;
    out.emissive = base * p.emissive;
    return out;
}
