use super::*;

fn validate(debug: bool) {
    let module = naga::front::wgsl::parse_str(&shader_source(debug))
        .expect("composed meshlet_deferred shader should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("composed meshlet_deferred shader should validate");
}

#[test]
fn deferred_shader_parses_and_validates() {
    validate(false);
}

/// Nothing compiles the debug variant until a debug view is opened, so
/// without this a break in it stays invisible until somebody looks.
#[test]
fn the_debug_variant_parses_and_validates() {
    validate(true);
}

/// The same guard the R64 path carries, for the reasons documented in
/// `material_pass::tests::the_game_shader_carries_no_debug_view` (#743).
#[test]
fn the_game_shader_carries_no_debug_view() {
    let production = shader_source(false);
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
    assert!(shader_source(true).contains("fn inti_shadow_debug("));
}

#[test]
fn screen_ubo_layout() {
    assert_eq!(std::mem::size_of::<ScreenUbo>(), 16);
}
