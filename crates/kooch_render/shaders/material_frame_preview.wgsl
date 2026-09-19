// material_frame_preview.wgsl — the frame the Shader Graph's preview runs a surface inside (#1159).
//
// 🔴 It declares exactly what the surface contract reads — `screen`, `inti`, `VertexOutput` — and
// nothing else. No visibility buffer, no Inti, no shadow pages: the same `surface` function the
// scene runs, rasterised over a primitive and lit by one key light of its own. That is what keeps
// a preview from needing half the renderer behind it.

struct PreviewCamera {
    view_proj: mat4x4<f32>,
}

// The fields `surface_input` reads off `screen`. Same names, same meaning as the real paths.
struct ScreenUniforms {
    material_id: u32,
    // `exp2(mip_bias)`. A preview samples at its own size, so it is always 1.
    mip_bias_scale: f32,
    // Seconds since the engine started, so a moving surface moves here too.
    time: f32,
    _pad: u32,
}

// Only `camera_position` is read by the contract, but the name has to be `inti` — that is what the
// prelude's `surface_input` reaches for.
struct IntiFrame {
    camera_position: vec3<f32>,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> camera: PreviewCamera;
@group(0) @binding(1) var<uniform> screen: ScreenUniforms;
@group(0) @binding(2) var<uniform> inti: IntiFrame;

// What the prelude's `surface_input` takes. The real paths reconstruct this from the visibility
// buffer; here the rasteriser interpolates it for us.
struct VertexOutput {
    world_position: vec3<f32>,
    world_normal: vec3<f32>,
    uv: vec2<f32>,
    ddx_uv: vec2<f32>,
    ddy_uv: vec2<f32>,
    world_tangent: vec4<f32>,
    material_id: u32,
    flags: u32,
}

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
}

struct VsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) world_tangent: vec4<f32>,
}

@vertex
fn vs_preview(in: VsIn) -> VsOut {
    var out: VsOut;
    // The mesh is already where it belongs: the camera orbits it, so there is no model matrix.
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.world_position = in.position;
    out.world_normal = in.normal;
    out.uv = in.uv;
    out.world_tangent = in.tangent;
    return out;
}

@fragment
fn fs_preview(in: VsOut) -> @location(0) vec4<f32> {
    var surf: VertexOutput;
    surf.world_position = in.world_position;
    surf.world_normal = normalize(in.world_normal);
    surf.world_tangent = in.world_tangent;
    surf.uv = in.uv;
    // Real quad derivatives, for once: this is an ordinary raster, not a visibility buffer.
    surf.ddx_uv = dpdx(in.uv);
    surf.ddy_uv = dpdy(in.uv);
    surf.material_id = screen.material_id;
    surf.flags = 0u;

    let input = surface_input(surf, in.clip_position.xy);
    let shaded = surface(input);
    // The cut the masked raster makes (#452); a clip of 0 keeps every fragment.
    if (shaded.alpha < shaded.alpha_clip) {
        discard;
    }
    if (SURFACE_UNLIT) {
        return vec4<f32>(shaded.emissive, 1.0);
    }

    let n = normalize(shaded.normal);
    let v = normalize(input.camera_position - input.world_position);
    let l = normalize(vec3<f32>(0.4, 0.7, 0.5));
    let h = normalize(l + v);

    // One key light, a little sky fill, and a highlight that narrows as roughness drops — enough
    // for metallic and roughness to read as themselves without any of Inti behind it.
    let diffuse = max(dot(n, l), 0.0);
    let fill = 0.12 + 0.10 * max(dot(n, vec3<f32>(0.0, 1.0, 0.0)), 0.0);
    let sharpness = mix(4.0, 160.0, 1.0 - clamp(shaded.roughness, 0.0, 1.0));
    let highlight = pow(max(dot(n, h), 0.0), sharpness) * (1.0 - clamp(shaded.roughness, 0.0, 1.0));
    let tint = mix(vec3<f32>(1.0), shaded.base_color, clamp(shaded.metallic, 0.0, 1.0));

    let lit = shaded.base_color * (diffuse + fill) + tint * highlight;
    return preview_over_checker(lit + shaded.emissive, shaded.alpha, in.clip_position.xy);
}

// A transparent surface over a checker, so its coverage shows (#452); anything else, as it is.
fn preview_over_checker(colour: vec3<f32>, alpha: f32, frag_coord: vec2<f32>) -> vec4<f32> {
    if (!SURFACE_TRANSPARENT) {
        return vec4<f32>(colour, 1.0);
    }
    let square = vec2<u32>(frag_coord / 12.0);
    let checker = select(0.18, 0.42, ((square.x + square.y) & 1u) == 0u);
    return vec4<f32>(mix(vec3<f32>(checker), colour, clamp(alpha, 0.0, 1.0)), 1.0);
}
