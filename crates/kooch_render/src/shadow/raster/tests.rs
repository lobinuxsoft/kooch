#[test]
fn shadow_shader_parses_and_validates() {
    let module =
        naga::front::wgsl::parse_str(super::SHADER_SOURCE).expect("shadow_depth.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("shadow_depth.wgsl should validate");
}

/// There is exactly one place biasing shadow depth, and it is the
/// shading pass. Two of them cannot both be tuned: each is set
/// against artifacts the other is already half-hiding.
#[test]
fn the_rasteriser_applies_no_depth_bias_of_its_own() {
    assert_eq!(super::DEPTH_BIAS.constant, 0);
    assert_eq!(super::DEPTH_BIAS.slope_scale, 0.0);
}

/// A silhouette is all a shadow is, so simplification error lands
/// in the outline where nothing hides it. Bevy applies no relaxation
/// to shadow views either — the budget is already in the cascade's
/// own texels, which is the term that makes a distant cascade ask
/// for less detail.
#[test]
fn a_shadow_gets_the_same_geometric_budget_as_the_camera() {
    assert_eq!(super::SHADOW_LOD_RELAXATION, 1.0);
}
