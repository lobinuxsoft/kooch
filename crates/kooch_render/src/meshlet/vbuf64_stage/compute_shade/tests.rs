use super::*;

/// 🔴 The default decides which shading path every existing capture,
/// screenshot and baseline was taken against. If this flips, three
/// sessions of measurements stop describing what the engine renders,
/// and nothing else in the suite would say so.
#[test]
fn compute_shading_is_off_unless_asked_for() {
    assert!(!parse_enabled(None));
    assert!(!parse_enabled(Some("")));
    assert!(!parse_enabled(Some("off")));
    assert!(!parse_enabled(Some("yes")));
    assert!(!parse_enabled(Some("ON")));
}

#[test]
fn the_spellings_a_measurement_run_would_use_all_work() {
    for raw in ["on", "1", "true"] {
        assert!(parse_enabled(Some(raw)), "KOOCH_COMPUTE_SHADING={raw}");
    }
}

/// The shader declares the shading target at a fixed binding, and a
/// group-0 layout that disagreed would fail at pipeline creation with a
/// message about binding counts rather than about this.
#[test]
fn the_colour_target_sits_past_the_contact_shadow_bindings() {
    assert!(COLOR_OUT_BINDING > MATERIAL_PASS_CONTACT_DEPTH_BINDING);
    assert!(
        MATERIAL_PBR_COMPUTE_BODY.contains(&format!("@group(0) @binding({COLOR_OUT_BINDING})"))
    );
}
