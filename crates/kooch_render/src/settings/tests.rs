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
