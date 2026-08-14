use super::*;

#[test]
fn substitution_leaves_no_placeholder_behind() {
    let src = inti_pbr_shader(5);
    assert!(
        !src.contains(GROUP_PLACEHOLDER),
        "a surviving placeholder is a shader that fails to parse at \
             pipeline creation, which is a runtime panic and not a test failure",
    );
    assert!(src.contains("@group(5) @binding(0)"));
    assert!(src.contains("@group(5) @binding(1)"));
    // The shadow bindings substitute too. The fourth is the one
    // worth pinning: it was written off as impossible on a
    // bind-group budget that is spent on *groups*, not on bindings
    // inside one, and if it silently disappears the blocker search
    // has nothing to sample and PCSS quietly becomes PCF again.
    assert!(src.contains("@group(5) @binding(2)"));
    assert!(src.contains("@group(5) @binding(3)"));
    assert!(src.contains("@group(5) @binding(4)"));
}

#[test]
fn the_template_is_not_valid_wgsl_on_its_own() {
    // Guards the reverse mistake: someone including the template
    // directly instead of calling the function would get a parse
    // error at pipeline creation. Better to state the contract.
    assert!(INTI_PBR_TEMPLATE.contains(GROUP_PLACEHOLDER));
}

/// The model calls `inti_contact_shadow` and never defines it, so
/// alone it is half a shader. That is deliberate — see
/// [`INTI_CONTACT_SHADOW_STUB`] — and this pins that the missing
/// half is exactly the one named, rather than something else having
/// gone missing.
#[test]
fn the_model_needs_a_contact_shadow_implementation_concatenated() {
    assert!(inti_pbr_shader(0).contains("inti_contact_shadow("));
    assert!(INTI_CONTACT_SHADOW_STUB.contains("fn inti_contact_shadow("));
}

#[test]
fn shading_model_parses_and_validates() {
    let module = naga::front::wgsl::parse_str(&format!(
        "{}\n{}",
        INTI_CONTACT_SHADOW_STUB,
        inti_pbr_shader(0)
    ))
    .expect("inti_pbr.wgsl should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("inti_pbr.wgsl should validate");
}

/// 🔴 The debug views had nothing validating them.
///
/// `inti_pbr.wgsl` is parsed above and `inti_debug.wgsl` was not, so a
/// typo in a view compiled for the first time when somebody opened it in
/// the editor — a shader panic on a dropdown selection, in a file whose
/// whole purpose is to be reached rarely. This concatenates the two the
/// way the editor's pipeline does and validates the result.
#[test]
fn the_debug_views_parse_and_validate() {
    let module = naga::front::wgsl::parse_str(&format!(
        "{}\n{}\n{}",
        INTI_CONTACT_SHADOW_STUB,
        inti_pbr_shader(0),
        inti_debug_shader(),
    ))
    .expect("inti_debug.wgsl should parse when concatenated after the model");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("inti_debug.wgsl should validate");
}

/// The stub and the real views must present the same call sites, or the
/// production pipeline stops compiling the moment a view is added.
///
/// The names are derived from the stub rather than restated, so a
/// function renamed in one file and not the other fails here instead of
/// in whichever build happens to be compiled next.
#[test]
fn the_stub_matches_the_views_it_replaces() {
    let views = inti_debug_shader();
    let mut found = 0;
    for line in INTI_DEBUG_STUB.lines() {
        let Some(signature) = line.strip_prefix("fn ") else {
            continue;
        };
        let name = signature.split('(').next().unwrap_or_default();
        assert!(
            views.contains(&format!("fn {name}(")),
            "the stub declares `{name}` and inti_debug.wgsl does not",
        );
        found += 1;
    }
    assert!(found >= 2, "the stub should declare both call sites");
}

/// 🔴 Zero means "never skip", and it is the default. A project that
/// never heard of #821 has to render exactly what it rendered before.
///
/// Reads the uniform rather than `SpecularFloor::default()`, which
/// consults the environment — a test that asserted on that would fail
/// for whoever happened to have the variable set while measuring.
#[test]
fn the_default_floor_keeps_every_specular() {
    assert_eq!(IntiFrame::default().specular_floor, 0.0);
}

/// A negative floor would mean nothing, and zero already means never.
#[test]
fn a_negative_floor_is_clamped() {
    assert_eq!(
        IntiFrame::default()
            .with_specular_floor(-5.0)
            .specular_floor,
        0.0
    );
    assert_eq!(
        IntiFrame::default()
            .with_specular_floor(120.0)
            .specular_floor,
        120.0
    );
}

/// The shader has to read the uniform for the control to do anything —
/// and to compare against the irradiance it already computed, not
/// against something it recomputes differently.
#[test]
fn the_shader_gates_on_the_floor() {
    let source = inti_pbr_shader(0);
    assert!(source.contains("inti.specular_floor"));
    assert!(source.contains("reach >= inti.specular_floor"));
}

/// `KOOCH_LIGHT_LIMIT`'s default has to be "every light", or every
/// capture taken without the variable would silently be measuring a
/// truncated scene (#824 follow-up).
#[test]
fn the_light_limit_defaults_to_all() {
    assert_eq!(IntiFrame::default().light_limit, 0);
    assert_eq!(LightLimit::default().0, 0);
}

#[test]
fn the_light_limit_reaches_the_uniform() {
    assert_eq!(IntiFrame::default().with_light_limit(3).light_limit, 3);
}

/// 🔴 Both shading paths must honour the cap. If only one did, the A/B
/// between them would be measuring the cap rather than the paths.
#[test]
fn both_paths_honour_the_light_limit() {
    let inti = crate::inti_pbr_shader(5);
    assert!(inti.contains("inti.light_limit"));
    assert!(
        kooch_render_compute_body().contains("inti.light_limit"),
        "the compute shading path ignores KOOCH_LIGHT_LIMIT",
    );
}

/// The compute body lives in `kooch_render`, which depends on this
/// crate — so it is read from the file rather than imported.
fn kooch_render_compute_body() -> &'static str {
    include_str!("../../kooch_render/shaders/material_pbr_compute.wgsl")
}
