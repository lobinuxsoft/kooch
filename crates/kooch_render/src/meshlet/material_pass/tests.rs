use super::*;
use crate::material::Shader;

/// A frame around the engine's own surface.
fn default_composed(frame: &str, debug: bool) -> String {
    let surface = Shader::default_surface();
    compose_material_shader(frame, &surface.params_wgsl(), &surface.source, debug)
}

/// 🔴 The Shader Graph's preview runs the SAME surface the scene runs, so the engine's own has to
/// compile inside the preview frame — which carries none of Inti, the contact-shadow march or the
/// visibility-buffer resolve (#1159).
#[test]
fn the_preview_frame_compiles() {
    let surface = Shader::default_surface();
    let composed = compose_preview_shader(&surface.params_wgsl(), &surface.source);
    validate(&composed, "the preview frame");
}

/// Reads and validates a surface the way the render does.
fn check(source: &str) -> Result<(), String> {
    let shader = Shader::parse(source).map_err(|e| e.to_string())?;
    validate_surface(&shader.params_wgsl(), &shader.source)
}

fn validate(source: &str, what: &str) {
    let module =
        naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{what} should parse: {e}"));
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    // `emit_to_string` rather than `{e}`: the Display impl of a
    // validation error is the headline only ("entry point invalid"),
    // and the line it happened on is in the span.
    validator
        .validate(&module)
        .unwrap_or_else(|e| panic!("{what} should validate:\n{}", e.emit_to_string(source)));
}

#[test]
fn resolve_material_depth_parses_and_validates() {
    validate(RESOLVE_MATERIAL_DEPTH_SHADER, "resolve_material_depth.wgsl");
}

/// Neither chunk validates alone — each references names the other
/// declares, which is the point of concatenating them. The composed
/// shader below is what actually has to parse.
#[test]
fn the_two_resolve_chunks_are_halves_of_one_shader() {
    assert!(VISIBILITY_BUFFER_RESOLVE_SHADER.contains("resolve_surface("));
    assert!(SURFACE_RECONSTRUCT_SHADER.contains("fn resolve_surface("));
}

#[test]
fn composed_default_material_parses_and_validates() {
    let composed = default_composed(MATERIAL_FRAGMENT_FRAME, false);
    validate(&composed, "composed default material shader");
}

/// The debug variant is a second pipeline the editor builds, and nothing
/// compiles it until somebody opens a debug view — so a break in it is
/// invisible until then unless a test compiles it here.
#[test]
fn the_debug_variant_parses_and_validates() {
    let composed = default_composed(MATERIAL_FRAGMENT_FRAME, true);
    validate(&composed, "composed default material shader (debug)");
}

#[test]
fn composed_compute_material_parses_and_validates() {
    let composed = default_composed(MATERIAL_COMPUTE_FRAME, false);
    validate(&composed, "composed compute material shader");
}

#[test]
fn the_compute_debug_variant_parses_and_validates() {
    let composed = default_composed(MATERIAL_COMPUTE_FRAME, true);
    validate(&composed, "composed compute material shader (debug)");
}

/// The dispatch derives its workgroup count from `SHADING_TILE_SIZE`, so
/// a shader that disagreed with it would leave a strip of the screen
/// unshaded — or run threads off the end of it — with nothing to say so.
#[test]
fn the_tile_size_matches_the_shader() {
    assert!(
        MATERIAL_COMPUTE_FRAME.contains(&format!("const TILE_SIZE: u32 = {SHADING_TILE_SIZE}u;")),
        "SHADING_TILE_SIZE and the shader's TILE_SIZE have diverged",
    );
    assert!(
        MATERIAL_COMPUTE_FRAME.contains(&format!(
            "@workgroup_size({SHADING_TILE_SIZE}, {SHADING_TILE_SIZE}, 1)"
        )),
        "the workgroup is not one thread per pixel of a tile",
    );
}

/// 🔴 The compute path exists to shade with the tile's lights, not to be a second copy of the
/// fragment path that quietly stopped doing it. Deleting the workgroup array would leave a shader
/// that still compiles and still renders correctly through the fallback — and buys nothing.
#[test]
fn the_compute_path_caches_the_tile_lights() {
    assert!(MATERIAL_COMPUTE_FRAME.contains("var<workgroup> tile_lights"));
    assert!(MATERIAL_COMPUTE_FRAME.contains("inti_lights[tile_lights[start + i]]"));
}

/// 🔴 The reason the variants exist (#743).
#[test]
fn the_game_shader_carries_no_debug_view() {
    let production = default_composed(MATERIAL_FRAGMENT_FRAME, false);
    for symbol in [
        "inti_shadow_debug",
        "inti_contact_shadow_debug_view",
        "inti_hsv_to_rgb",
    ] {
        assert!(
            !production.contains(symbol),
            "`{symbol}` is in the production shader; it belongs in inti_debug.wgsl",
        );
    }
    assert!(
        default_composed(MATERIAL_FRAGMENT_FRAME, true).contains("fn inti_shadow_debug("),
        "the debug variant is supposed to be the one that has them",
    );
}

/// The shipped surface passes the same gate an author's does.
#[test]
fn the_default_surface_is_valid() {
    check(DEFAULT_SURFACE_SHADER).unwrap();
}

/// A custom body composes into both frames: this one ignores its maps and paints red.
#[test]
fn a_custom_surface_validates() {
    let red = "fn surface(input: SurfaceInput) -> SurfaceOutput {
        var out: SurfaceOutput;
        out.base_color = vec3<f32>(1.0, 0.0, 0.0);
        out.normal = normalize(input.world_normal);
        out.metallic = 0.0;
        out.roughness = 0.5;
        out.emissive = vec3<f32>(0.0);
        return out;
    }";
    check(red).unwrap();
}

const UNLIT_RED: &str = "// kind: unlit
fn unlit(input: SurfaceInput) -> UnlitOutput {
    return UnlitOutput(vec3<f32>(1.0, 0.0, 0.0), 1.0);
}";

/// An unlit body composes into both frames and the preview.
#[test]
fn an_unlit_shader_validates() {
    check(UNLIT_RED).unwrap();
    let shader = Shader::parse(UNLIT_RED).unwrap();
    validate_preview(&shader.params_wgsl(), &shader.source).unwrap();
}

/// The frames call `surface`; an unlit file only gets one from its kind line.
#[test]
fn an_unlit_body_needs_its_kind() {
    assert!(check(UNLIT_RED.trim_start_matches("// kind: unlit\n")).is_err());
}

/// The line reported is the surface file's, not the composed shader's.
#[test]
fn a_broken_surface_names_its_line() {
    let broken = "fn surface(input: SurfaceInput) -> SurfaceOutput {\n    let x = ;\n}";
    let error = check(broken).unwrap_err();
    assert!(error.starts_with("line 2: "), "{error}");
}

/// New Shader's template: its header parses, its generated code and body validate on both frames.
#[test]
fn the_new_shader_template_is_valid() {
    let shader = Shader::parse(NEW_SURFACE_SHADER).unwrap();
    assert_eq!(shader.params.len(), 9);
    check(NEW_SURFACE_SHADER).unwrap();
}

/// A declared scalar reads its own offset in `material_values`.
#[test]
fn params_generate_their_reads() {
    let shader = Shader::parse("struct SurfaceParams { a: f32, b: vec2<f32> }").unwrap();
    let wgsl = shader.params_wgsl();
    assert!(wgsl.contains("p.a = material_values[base + 0u];"), "{wgsl}");
    assert!(
        wgsl.contains("p.b = vec2<f32>(material_values[base + 1u], material_values[base + 2u]);"),
        "{wgsl}"
    );
}

/// A surface with a texture and parameters, written like the docs show, validates on both frames.
#[test]
fn a_parameterised_surface_validates() {
    check(
        "struct SurfaceParams { tint: vec4<f32> } // @color
        var detail: texture_2d<f32>;
        fn surface(input: SurfaceInput) -> SurfaceOutput {
            let p = surface_params(input.material_id);
            var out: SurfaceOutput;
            out.base_color = sample_surface(detail, input, input.uv, vec2(1.0)).rgb * p.tint.rgb;
            out.normal = normalize(input.world_normal);
            out.roughness = 0.5;
            return out;
        }",
    )
    .unwrap();
}

/// The rim-glow example from the Shaders guide: defaults, hints, a texture and the camera.
#[test]
fn the_rim_example_validates() {
    check(
        "struct SurfaceParams {
            tint: vec4<f32>,        // @color
            rim_color: vec4<f32>,   // @color
            rim_power: f32,         // @range(0.5, 8)
        }
        const SURFACE_DEFAULTS = SurfaceParams(vec4(1.0), vec4(0.2, 0.8, 1.0, 1.0), 3.0);
        var albedo: texture_2d<f32>;
        fn surface(input: SurfaceInput) -> SurfaceOutput {
            let p = surface_params(input.material_id);
            let n = normalize(input.world_normal);
            let to_camera = normalize(input.camera_position - input.world_position);
            let rim = pow(1.0 - saturate(dot(n, to_camera)), p.rim_power);
            var out: SurfaceOutput;
            out.base_color = sample_surface(albedo, input, input.uv, vec2(1.0)).rgb * p.tint.rgb;
            out.normal = n;
            out.roughness = 0.6;
            out.emissive = p.rim_color.rgb * rim;
            return out;
        }",
    )
    .unwrap();
}

const VIGNETTE: &str = "// kind: post_process
fn post_process(input: SurfaceInput) -> vec4<f32> {
    let scene = sample_scene(input.uv);
    let edge = 1.0 - length(input.uv - vec2<f32>(0.5)) * 1.4;
    return vec4<f32>(scene.rgb * clamp(edge, 0.0, 1.0), scene.a);
}";

/// A post-process composes into its own frame, and reads the scene through the frame's sampler.
#[test]
fn a_post_process_validates() {
    let shader = Shader::parse(VIGNETTE).unwrap();
    assert_eq!(shader.kind, crate::material::ShaderKind::PostProcess);
    validate_post(&shader.params_wgsl(), &shader.source).unwrap();
}

/// Its parameters are the material's, as a surface's are.
#[test]
fn a_post_process_reads_its_params() {
    let source = "// kind: post_process
struct SurfaceParams { strength: f32 } // @range(0, 2)
fn post_process(input: SurfaceInput) -> vec4<f32> {
    let p = surface_params(input.material_id);
    return sample_scene(input.uv) * p.strength;
}";
    let shader = Shader::parse(source).unwrap();
    assert_eq!(shader.params.len(), 1);
    validate_post(&shader.params_wgsl(), &shader.source).unwrap();
}

/// The line reported is the file's, not the composed shader's.
#[test]
fn a_broken_post_names_its_line() {
    let broken = "// kind: post_process\nfn post_process(input: SurfaceInput) -> vec4<f32> {\n    let x = ;\n}";
    let shader = Shader::parse(broken).unwrap();
    let error = validate_post(&shader.params_wgsl(), &shader.source).unwrap_err();
    assert!(error.starts_with("line 3: "), "{error}");
}
