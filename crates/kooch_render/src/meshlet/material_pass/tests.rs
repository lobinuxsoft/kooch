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
    let composed = compose_material_shader(MATERIAL_PBR_DEFAULT_BODY, false);
    validate(&composed, "composed default material shader");
}

/// The debug variant is a second pipeline the editor builds, and nothing
/// compiles it until somebody opens a debug view — so a break in it is
/// invisible until then unless a test compiles it here.
#[test]
fn the_debug_variant_parses_and_validates() {
    let composed = compose_material_shader(MATERIAL_PBR_DEFAULT_BODY, true);
    validate(&composed, "composed default material shader (debug)");
}

/// 🔴 The reason the variants exist (#743).
///
/// A branch nothing takes is still code the shader carries: register
/// allocation is worst-case over the entry point, so a cascade sample
/// and a screen-space march parked behind `if (debug_mode == …)` still
/// cost occupancy — which is the whole of an integrated GPU's latency
/// hiding, on a budget of 13.9 ms at 10 W.
///
/// If this fails, a debug view leaked back into the shader every shipped
/// game runs, and nothing else will say so.
#[test]
fn the_game_shader_carries_no_debug_view() {
    let production = compose_material_shader(MATERIAL_PBR_DEFAULT_BODY, false);
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
        compose_material_shader(MATERIAL_PBR_DEFAULT_BODY, true).contains("fn inti_shadow_debug("),
        "the debug variant is supposed to be the one that has them",
    );
}
