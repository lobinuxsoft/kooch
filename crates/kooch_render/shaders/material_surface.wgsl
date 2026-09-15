// material_surface.wgsl — what a surface shader reads and returns (#1157). Composed after the
// resolve helpers and before the surface body, so both shading frames share one contract.

struct MaterialParams {
    base_color: vec4<f32>,
    // x metallic, y roughness, z emissive, w pad.
    metallic_roughness_emissive_pad: vec4<f32>,
    texture_indices: vec4<u32>,
    // xy tiling, zw offset. Also declared in `meshlet_deferred.wgsl`; a test reads both.
    uv_scale_offset: vec4<f32>,
}

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;
// A shader's declared scalars, `MAX_PARAM_SCALARS` per material; read through `surface_params` (#1158).
@group(2) @binding(1) var<storage, read> material_values: array<f32>;

@group(4) @binding(0) var albedo_tex: texture_2d<f32>;
@group(4) @binding(1) var normal_tex: texture_2d<f32>;
@group(4) @binding(2) var metal_rough_tex: texture_2d<f32>;
@group(4) @binding(3) var material_sampler: sampler;
// A shader's fourth texture parameter.
@group(4) @binding(4) var extra_tex: texture_2d<f32>;

/// The reconstructed surface point a shader shades.
struct SurfaceInput {
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    world_tangent: vec4<f32>,
    uv: vec2<f32>,
    // Analytical, in mesh uv units: quad derivatives are wrong on a visibility buffer.
    ddx_uv: vec2<f32>,
    ddy_uv: vec2<f32>,
    // `exp2(mip_bias)`; multiply the derivatives by it before sampling (#881).
    mip_bias_scale: f32,
    frag_coord: vec2<f32>,
    // Index into `materials`.
    material_id: u32,
}

/// What Inti lights.
struct SurfaceOutput {
    base_color: vec3<f32>,
    // World space, normalised.
    normal: vec3<f32>,
    metallic: f32,
    roughness: f32,
    // Radiance added after lighting.
    emissive: vec3<f32>,
}

fn surface_input(surf: VertexOutput, frag_coord: vec2<f32>) -> SurfaceInput {
    var input: SurfaceInput;
    input.world_position = surf.world_position;
    input.world_normal = surf.world_normal;
    input.world_tangent = surf.world_tangent;
    input.uv = surf.uv;
    input.ddx_uv = surf.ddx_uv;
    input.ddy_uv = surf.ddy_uv;
    input.mip_bias_scale = screen.mip_bias_scale;
    input.frag_coord = frag_coord;
    input.material_id = screen.material_id;
    return input;
}
