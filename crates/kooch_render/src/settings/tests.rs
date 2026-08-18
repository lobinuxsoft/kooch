use super::*;

#[test]
fn defaults_match_what_the_engine_does_without_a_file() {
    let settings = RenderSettings::default();
    assert_eq!(settings.camera(), PhysicalCamera::default());
    assert_eq!(settings.ambient(), AmbientLight::default());
}

/// A settings file written by an older engine, or by hand with one
/// line in it, must load. Failing shut on an unknown or absent field
/// would make the asset a liability rather than a convenience.
#[test]
fn a_partial_file_fills_the_rest_from_defaults() {
    let loader = RenderSettingsLoader;
    let path = std::path::Path::new("project.rendersettings");
    let mut ctx = LoadContext { path };
    let parsed = loader
        .load(b"(aperture_f_stops: 1.4)", &mut ctx)
        .expect("a one-field file should load");
    assert_eq!(parsed.aperture_f_stops, 1.4);
    assert_eq!(
        parsed.sensitivity_iso,
        RenderSettings::default().sensitivity_iso
    );
}

#[test]
fn an_empty_file_is_entirely_defaults() {
    let loader = RenderSettingsLoader;
    let path = std::path::Path::new("project.rendersettings");
    let mut ctx = LoadContext { path };
    let parsed = loader
        .load(b"()", &mut ctx)
        .expect("an empty record should load");
    assert_eq!(parsed, RenderSettings::default());
}

#[test]
fn round_trips_through_ron() {
    let mut settings = RenderSettings::default();
    settings.aperture_f_stops = 2.0;
    settings.ambient_intensity = 42.0;
    let text = to_ron(&settings).expect("serialises");
    let back: RenderSettings = ron::from_str(&text).expect("deserialises");
    assert_eq!(back, settings);
}

#[test]
fn nonsense_is_refused_rather_than_defaulted() {
    let loader = RenderSettingsLoader;
    let path = std::path::Path::new("project.rendersettings");
    let mut ctx = LoadContext { path };
    assert!(loader.load(b"this is not ron", &mut ctx).is_err());
}

/// Every field carries a tooltip, because the whole reason to open
/// this asset is not knowing what the numbers mean.
#[test]
fn every_field_explains_itself() {
    let settings = RenderSettings::default();
    let missing: Vec<_> = settings
        .reflect_fields()
        .iter()
        .filter(|m| m.doc.trim().is_empty())
        .map(|m| m.name)
        .collect();
    assert!(missing.is_empty(), "fields with no tooltip: {missing:?}");
}

#[test]
fn the_unit_of_each_number_is_stated() {
    let settings = RenderSettings::default();
    let doc = |name: &str| {
        settings
            .reflect_fields()
            .iter()
            .find(|m| m.name == name)
            .map(|m| m.doc)
            .unwrap_or("")
    };
    assert!(doc("shutter_speed_s").contains("SECONDS"));
    assert!(doc("ambient_intensity").contains("LUX"));
}

/// 🔴 A `.rendersettings` written before #826 was removed must still
/// load, and quietly.
///
/// This is the removal's only real hazard. Deleting a field from a
/// serialized asset is not like deleting a function: the compiler cannot
/// see the files already on disk, and every project that ever opened the
/// Shading group has `light_samples` written into its own
/// `project.rendersettings` — roll-a-ball's said `light_samples: 4` when
/// this landed. If RON rejected the unknown key the asset would fail to
/// load, and the project would come up with default exposure, default
/// shadows and default everything with nothing naming the cause.
///
/// Goes through the real loader rather than `ron::from_str`, because the
/// loader is what a project actually hits.
#[test]
fn a_settings_file_with_the_removed_field_still_loads() {
    let loader = RenderSettingsLoader;
    let path = std::path::Path::new("project.rendersettings");
    let mut ctx = LoadContext { path };
    let parsed = loader
        .load(
            b"(aperture_f_stops: 2.8, light_samples: 4, compute_shading: true)",
            &mut ctx,
        )
        .expect(
            "a .rendersettings carrying the removed `light_samples` key failed to load. \
             Every project that touched the Shading group has one, and a project whose \
             settings fail to load renders with defaults and says nothing.",
        );
    assert!(
        parsed.compute_shading,
        "the file loaded but the field after the removed key was not read",
    );
    assert_eq!(parsed.aperture_f_stops, 2.8);
}

/// 🔴 The resolve is gated on the compute path, and the enum must not
/// have quietly dropped that gate: the jitter would stay on with
/// nothing to integrate it, which shimmers and reads as the technique
/// being broken rather than inapplicable.
#[test]
fn the_fragment_path_gets_no_technique() {
    let settings = "(upscale: 1, compute_shading: false)";
    let parsed: RenderSettings = ron::from_str(settings).expect("should load");
    assert_eq!(parsed.technique(), crate::quality::UpscaleTechnique::Taa);
    assert!(
        !parsed.temporal().enabled(),
        "the fragment path must resolve to no technique whatever the file asks for",
    );
}

/// 🔴 A file written before `temporal_aa` was deleted must still load.
///
/// Same hazard the removal of `light_samples` had, and the same test:
/// the compiler cannot see the files already on disk, and every project
/// that ever opened the Temporal group has `temporal_aa` written into
/// its own `project.rendersettings`. If RON rejected the unknown key the
/// asset would fail to load and the project would render with engine
/// defaults for EVERYTHING, not just for this one setting.
///
/// The value itself is gone on purpose — the owner's call, since the
/// dropdown replaced it and no project outside this repo predates it.
/// What must not happen is the file failing.
#[test]
fn a_file_naming_the_deleted_toggle_still_loads() {
    let parsed: RenderSettings =
        ron::from_str("(aperture_f_stops: 2.8, temporal_aa: true, upscale: 1)")
            .expect("an unknown key must not fail the load");
    assert_eq!(parsed.aperture_f_stops, 2.8);
    assert_eq!(
        parsed.technique(),
        crate::quality::UpscaleTechnique::Taa,
        "the field after the deleted key was not read",
    );
}
