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
}

@group(2) @binding(0) var<storage, read> materials: array<MaterialParams>;

@group(4) @binding(0) var albedo_tex: texture_2d<f32>;
@group(4) @binding(1) var normal_tex: texture_2d<f32>;
@group(4) @binding(2) var metal_rough_tex: texture_2d<f32>;
@group(4) @binding(3) var material_sampler: sampler;

// `MeshletDebugMode::Normals`. Kept in sync with the Rust enum by the
// test in `debug.rs` that pins the discriminant.
const DEBUG_MODE_NORMALS: u32 = 11u;
// `MeshletDebugMode::ShadowCascades`, pinned by the same test.
const DEBUG_MODE_SHADOW_CASCADES: u32 = 12u;

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

    // The world-space normal as colour. Until #441 this WAS the shading
    // model; it is now one entry in the debug dropdown, which is what a
    // debug view was always supposed to be.
    if (screen.debug_mode == DEBUG_MODE_NORMALS) {
        return vec4<f32>(world_n * 0.5 + 0.5, 1.0);
    }
    if (screen.debug_mode == DEBUG_MODE_SHADOW_CASCADES) {
        let vd = dot(surf.world_position - inti.camera_position, inti.camera_forward);
        return vec4<f32>(inti_shadow_debug(surf.world_position, vd), 1.0);
    }

    // glTF packing: green is roughness, blue is metallic. The 1×1
    // fallback is white, so a material with no map multiplies its
    // scalars by 1 and there is no branch.
    let mr = textureSampleGrad(
        metal_rough_tex, material_sampler, surf.uv, surf.ddx_uv, surf.ddy_uv);
    let metallic = mat.metallic_roughness_emissive_pad.x * mr.b;
    let roughness = mat.metallic_roughness_emissive_pad.y * mr.g;

    var radiance = inti_shade(surf.world_position, world_n, base, metallic, roughness);
    // Emissive is radiance the surface produces rather than reflects, so
    // it joins before tonemapping and ignores every light in the scene.
    radiance += base * mat.metallic_roughness_emissive_pad.z;

    return vec4<f32>(inti_tonemap(radiance), 1.0);
}
