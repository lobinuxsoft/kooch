use super::*;

/// 🔴 An unset variable has to read as `None`, not as `Some(false)`.
///
/// The variable now sits ON TOP of the project's `.rendersettings`
/// (#830). If "unset" meant "off", every project would be forced onto
/// the fragment path by an environment nobody configured, and the
/// settings asset would appear to have a compute-shading switch that
/// does nothing.
#[test]
fn an_unset_variable_says_nothing() {
    assert_eq!(parse_enabled(None), None);
    // …and so does a spelling the parser does not recognise. A typo
    // during a measurement run must not silently change which path is
    // being measured.
    assert_eq!(parse_enabled(Some("")), None);
    assert_eq!(parse_enabled(Some("yes")), None);
    assert_eq!(parse_enabled(Some("ON")), None);
}

#[test]
fn the_spellings_a_measurement_run_would_use_all_work() {
    for raw in ["on", "1", "true"] {
        assert_eq!(
            parse_enabled(Some(raw)),
            Some(true),
            "KOOCH_COMPUTE_SHADING={raw}",
        );
    }
    // Both directions, because the asset's default is now `true`: a
    // capture that needs the fragment path has to be able to ask for it.
    for raw in ["off", "0", "false"] {
        assert_eq!(
            parse_enabled(Some(raw)),
            Some(false),
            "KOOCH_COMPUTE_SHADING={raw}",
        );
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
