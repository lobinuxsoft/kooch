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
