use super::*;

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
    let composed =
        compose_material_shader(MATERIAL_FRAGMENT_FRAME, "", DEFAULT_SURFACE_SHADER, false);
    validate(&composed, "composed default material shader");
}

/// The debug variant is a second pipeline the editor builds, and nothing
/// compiles it until somebody opens a debug view — so a break in it is
/// invisible until then unless a test compiles it here.
#[test]
fn the_debug_variant_parses_and_validates() {
    let composed =
        compose_material_shader(MATERIAL_FRAGMENT_FRAME, "", DEFAULT_SURFACE_SHADER, true);
    validate(&composed, "composed default material shader (debug)");
}

#[test]
fn composed_compute_material_parses_and_validates() {
    let composed =
        compose_material_shader(MATERIAL_COMPUTE_FRAME, "", DEFAULT_SURFACE_SHADER, false);
    validate(&composed, "composed compute material shader");
}

#[test]
fn the_compute_debug_variant_parses_and_validates() {
    let composed =
        compose_material_shader(MATERIAL_COMPUTE_FRAME, "", DEFAULT_SURFACE_SHADER, true);
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
    let production =
        compose_material_shader(MATERIAL_FRAGMENT_FRAME, "", DEFAULT_SURFACE_SHADER, false);
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
        compose_material_shader(MATERIAL_FRAGMENT_FRAME, "", DEFAULT_SURFACE_SHADER, true)
            .contains("fn inti_shadow_debug("),
        "the debug variant is supposed to be the one that has them",
    );
}

/// The shipped surface passes the same gate an author's does.
#[test]
fn the_default_surface_is_valid() {
    validate_surface("", DEFAULT_SURFACE_SHADER).unwrap();
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
    validate_surface("", red).unwrap();
}

/// The line reported is the surface file's, not the composed shader's.
#[test]
fn a_broken_surface_names_its_line() {
    let broken = "fn surface(input: SurfaceInput) -> SurfaceOutput {\n    let x = ;\n}";
    let error = validate_surface("", broken).unwrap_err();
    assert!(error.starts_with("line 2: "), "{error}");
}

/// A binding of its own would not match the pipeline layout.
#[test]
fn a_surface_cannot_bind() {
    let bound =
        format!("@group(4) @binding(9) var extra: texture_2d<f32>;\n{DEFAULT_SURFACE_SHADER}");
    assert!(validate_surface("", &bound).is_err());
}

/// New Shader's template: its header parses, its generated code and body validate on both frames.
#[test]
fn the_new_shader_template_is_valid() {
    let shader = crate::material::Shader::parse(NEW_SURFACE_SHADER).unwrap();
    assert_eq!(shader.params.len(), 9);
    validate_surface(&shader.params_wgsl(), &shader.source).unwrap();
}

/// A declared scalar reads its own offset in `material_values`.
#[test]
fn params_generate_their_reads() {
    let shader = crate::material::Shader::parse("// param a: float\n// param b: vec2").unwrap();
    let wgsl = shader.params_wgsl();
    assert!(wgsl.contains("p.a = material_values[base + 0u];"), "{wgsl}");
    assert!(
        wgsl.contains("p.b = vec2<f32>(material_values[base + 1u], material_values[base + 2u]);"),
        "{wgsl}"
    );
}
