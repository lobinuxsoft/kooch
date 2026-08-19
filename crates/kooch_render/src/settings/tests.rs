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
    let mut ctx = LoadContext::new(path);
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
    let mut ctx = LoadContext::new(path);
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
    let mut ctx = LoadContext::new(path);
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
    let mut ctx = LoadContext::new(path);
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

/// 🔴 `render_scale` must not be offered for a technique that ignores
/// it.
///
/// It is already forced to 100 for `None` and `TAA`, so the control did
/// nothing — and a control that silently does nothing is worse than an
/// absent one, because it reads as "I tried the setting and it did not
/// help". Reported by the owner, who set it under TAA and reasonably
/// expected it to apply.
///
/// Pinned as the condition's VALUES rather than by rendering anything:
/// the enum's numbers are serialised into user projects and are
/// append-only, so a variant renumbered without updating this would
/// show the control for the wrong technique.
#[test]
fn the_scale_is_offered_only_where_it_acts() {
    let shown: Vec<u32> = UPSCALES_WHEN.values.iter().map(|v| *v as u32).collect();
    for value in 0..4u32 {
        let technique = crate::quality::UpscaleTechnique::from_asset(value);
        assert_eq!(
            shown.contains(&value),
            technique.upscales(),
            "technique {technique:?} (asset value {value}) upscales={} but the inspector \
             condition says shown={}",
            technique.upscales(),
            shown.contains(&value),
        );
    }
}

/// The anisotropy in the asset reaches the settings the renderer reads.
///
/// ⚠️ Its own test because the GPU one cannot cover it: that rig
/// registers its material by hand, so the texture sync — which is what
/// carries this number from `ShadingSettings` to the sampler — has no
/// snapshots to run on. Two claims, two tests: this one is "the file is
/// read", the GPU one is "the sampler does something".
#[test]
fn the_anisotropy_travels_from_the_asset() {
    let ron = r#"(compute_shading: true, anisotropy: 8)"#;
    let path = std::path::Path::new("look.rendersettings");
    let mut ctx = LoadContext::new(path);
    let parsed = RenderSettingsLoader.load(ron.as_bytes(), &mut ctx).unwrap();
    assert_eq!(parsed.anisotropy, 8);
    assert_eq!(parsed.shading().anisotropy, 8);
}

/// 🔴 And a number hardware does not implement is clamped, not passed on.
///
/// `anisotropy_clamp` is a `u16` the sampler validates: zero is not a
/// legal value and wgpu rejects the descriptor outright, which would
/// take down every material in the project over one hand-edited line.
#[test]
fn an_impossible_anisotropy_is_clamped() {
    for (written, expected) in [(0u32, 1u16), (3, 3), (64, 16), (100_000, 16)] {
        let ron = format!("(anisotropy: {written})");
        let path = std::path::Path::new("look.rendersettings");
        let mut ctx = LoadContext::new(path);
        let parsed = RenderSettingsLoader.load(ron.as_bytes(), &mut ctx).unwrap();
        assert_eq!(
            parsed.shading().anisotropy,
            expected,
            "anisotropy {written} reached the sampler as {}",
            parsed.shading().anisotropy,
        );
    }
}
