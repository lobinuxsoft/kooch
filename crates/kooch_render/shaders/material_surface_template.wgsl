// kind: surface
#import kooch::surface

// The material's fields: each member is one in the Inspector. Hints after `//` are optional.
struct SurfaceParams {
    base_color: vec4<f32>,  // @color
    metallic: f32,          // @range(0, 1)
    roughness: f32,         // @range(0, 1)
    emissive: f32,          // @range(0, 10)
    uv_scale: vec2<f32>,
    uv_offset: vec2<f32>,
}

// What a new material starts with. Without it every field starts at zero.
const SURFACE_DEFAULTS = SurfaceParams(
    vec4(1.0),        // base_color
    0.0,              // metallic
    0.5,              // roughness
    0.0,              // emissive
    vec2(1.0),        // uv_scale
    vec2(0.0),        // uv_offset
);

// The material's textures, four at most. The engine binds them.
var albedo: texture_2d<f32>;           // @default(white)
var normal_map: texture_2d<f32>;       // @default(normal)
var metal_roughness: texture_2d<f32>;  // @default(white)

fn surface(input: SurfaceInput) -> SurfaceOutput {
    let p = surface_params(input.material_id);
    let uv = input.uv * p.uv_scale + p.uv_offset;

    let base = sample_surface(albedo, input, uv, p.uv_scale).rgb * p.base_color.rgb;
    let n_ts = sample_surface(normal_map, input, uv, p.uv_scale).xyz * 2.0 - 1.0;
    let n = normalize(input.world_normal);
    let t = normalize(input.world_tangent.xyz);
    let b = cross(n, t) * input.world_tangent.w;
    // glTF packing: green is roughness, blue is metallic.
    let mr = sample_surface(metal_roughness, input, uv, p.uv_scale);

    var out: SurfaceOutput;
    out.base_color = base;
    out.normal = normalize(mat3x3<f32>(t, b, n) * n_ts);
    out.metallic = p.metallic * mr.b;
    out.roughness = p.roughness * mr.g;
    out.emissive = base * p.emissive;
    return out;
}
