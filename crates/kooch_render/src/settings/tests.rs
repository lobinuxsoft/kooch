use super::*;

/// 🔴 The asset is the SOURCE of these values, not a mirror of them.
#[test]
fn unchosen_fields_fall_back() {
    let settings = RenderSettings::default();
    // Untouched, so still the camera's own.
    assert_eq!(settings.camera(), PhysicalCamera::default());
    let ambient = settings.ambient();
    assert_eq!(ambient.ground_color, AmbientLight::default().ground_color);
    assert_eq!(ambient.intensity, AmbientLight::default().intensity);
    // Chosen, and deliberately not the fallback.
    assert_ne!(ambient.sky_color, AmbientLight::default().sky_color);
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

/// 🔴 A `.rendersettings` written before #826 was removed must still load, and quietly.
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

/// 🔴 The resolve is gated on the compute path, and the enum must not have quietly dropped that
/// gate: the jitter would stay on with nothing to integrate it, which shimmers and reads as the
/// technique being broken rather than inapplicable.
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

/// 🔴 `render_scale` must not be offered for a technique that ignores it.
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

/// 🔴 `apply` publishes the presentation, and the staleness check in `apply_render_settings_system`
/// compares it.
#[test]
fn apply_publishes_the_presentation() {
    let mut resources = kooch_core::resource::Resources::new();
    let mut settings = RenderSettings::default();
    settings.vsync = false;
    settings.apply(&mut resources);
    assert_eq!(
        resources.get::<crate::quality::Presentation>(),
        Some(&crate::quality::Presentation { vsync: false }),
    );
}

/// 🔴 What a file written before this field existed silently becomes, which `settings.rs` argues
/// about at length for `compute_shading`: a serde default is not a recommendation.
#[test]
fn a_missing_vsync_defaults() {
    let parsed: RenderSettings = ron::from_str("(sharpening: 0)").expect("partial file");
    assert_eq!(parsed.vsync, RenderSettings::default().vsync);
}

/// 🔴 What a file written before this field existed silently becomes. Every `.rendersettings` on
/// every disk predates `window_mode`, and taking the display is not something a project opts into
/// by upgrading the engine.
#[test]
fn a_missing_mode_defaults() {
    let parsed: RenderSettings = ron::from_str("(sharpening: 0)").expect("partial file");
    assert_eq!(
        parsed.window_mode(),
        RenderSettings::default().window_mode()
    );
}

/// `apply` publishes it and `apply_render_settings_system` compares it —
/// a resource missing from either place makes the setting do nothing at
/// all, silently.
#[test]
fn apply_publishes_the_window_mode() {
    let mut resources = kooch_core::resource::Resources::new();
    let mut settings = RenderSettings::default();
    settings.window_mode = 2;
    settings.apply(&mut resources);
    assert_eq!(
        resources.get::<kooch_core::window_mode::WindowMode>(),
        Some(&kooch_core::window_mode::WindowMode::Fullscreen),
    );
}

#[test]
fn virtual_shadows_reaches_the_published_settings() {
    // 🔴 The regression this exists for shipped a whole feature inert. `virtual_shadows` was read at
    // the call site off `RenderSettings`, which `apply` never inserts as a `Resources` value, so
    // the lookup returned `None` in every build and the fallback turned the pages off.
    let settings = RenderSettings {
        virtual_shadows: true,
        shadow_density: 50,
        shadow_pool_pages: 4096,
        ..Default::default()
    };
    let published = settings.shadows();
    assert!(published.virtual_pages);
    assert_eq!(published.page_density, 50);
    assert_eq!(published.pool_pages, 4096);
}

#[test]
fn shadow_softness_reaches_the_published_settings() {
    // Same class as `virtual_shadows_reaches_the_published_settings`:
    // a knob that stops at the asset ships a filter nobody can widen.
    let settings = RenderSettings {
        shadow_softness: 3,
        ..Default::default()
    };
    assert_eq!(settings.shadows().page_softness, 3);
    assert_eq!(RenderSettings::default().shadow_softness, 3);
}

#[test]
fn shadow_min_pixels_reaches_the_settings() {
    // Same class again: a gate that stops at the asset ships every
    // light casting forever.
    let settings = RenderSettings {
        shadow_min_pixels: 32,
        ..Default::default()
    };
    assert_eq!(settings.shadows().page_min_pixels, 32);
    // The default gates only what nobody could resolve anyway.
    assert_eq!(RenderSettings::default().shadow_min_pixels, 8);
}

#[test]
fn the_lod_target_reaches_the_frame() {
    // The same class one more time, and the one with the widest blast radius: this is the
    // quality-against-cost lever of a meshlet renderer, and only the editor ever inserted the
    // resource the frame reads.
    let settings = RenderSettings {
        meshlet_lod_error: 4.0,
        ..Default::default()
    };
    assert_eq!(settings.meshlet_lod().target_error_pixels, 4.0);
    assert_eq!(RenderSettings::default().meshlet_lod_error, 0.5);
    assert_eq!(
        RenderSettings::default().meshlet_lod().target_error_pixels,
        0.5,
        "the default has to be what the engine ran at before the setting existed",
    );
    // Zero would mean no level is ever fine enough and the cull emits nothing, which is a black
    // screen rather than a coarse one. A settings file is a text file, and the Inspector's range
    // does not constrain what someone types into one.
    let zero = RenderSettings {
        meshlet_lod_error: 0.0,
        ..Default::default()
    };
    assert!(zero.meshlet_lod().target_error_pixels > 0.0);
    let huge = RenderSettings {
        meshlet_lod_error: 9999.0,
        ..Default::default()
    };
    assert_eq!(huge.meshlet_lod().target_error_pixels, 8.0);
}

#[test]
fn the_lod_range_reaches_the_inspector() {
    // The declaration travels a long way — attribute, macro, `FieldMeta` — and every step of it is
    // silent when it drops something: the Inspector simply draws the unbounded drag it drew before,
    // and the bound that stops a zero reaching the cull is quietly not there.
    use kooch_ecs::reflect::Reflect;
    let meta = RenderSettings::default()
        .reflect_fields()
        .iter()
        .find(|f| f.name == "meshlet_lod_error")
        .expect("the field is reflected");
    let range = meta.range.expect("the field declares a range");
    assert_eq!(range.min, 0.01, "zero would emit no geometry at all");
    assert_eq!(range.max, 8.0);
    assert_eq!(range.step, 0.01);
}

#[test]
fn the_shadow_bias_reaches_the_settings() {
    // Two WGSL constants, unreachable from any project, deciding
    // whether a shadow exists at all — the same class as the LOD target
    // above and as `virtual_shadows` before it.
    let settings = RenderSettings {
        shadow_normal_bias: 0.5,
        shadow_depth_bias: 0.05,
        shadow_bias_max: 0.04,
        shadow_bias_slope: 2.5,
        ..Default::default()
    };
    let shadows = settings.shadows();
    assert_eq!(shadows.page_normal_bias, 0.5);
    assert_eq!(shadows.page_depth_bias, 0.05);
    assert_eq!(shadows.page_bias_max, 0.04);
    assert_eq!(shadows.page_bias_slope, 2.5);

    let default = RenderSettings::default();
    assert_eq!(default.shadow_normal_bias, 4.0);
    assert_eq!(default.shadow_depth_bias, 0.2);
    assert_eq!(
        default.shadow_bias_max, 0.5,
        "the cap is on by default now; the shadow track measured this pair",
    );
    // ⚠️ OFF, and this one is worth reading twice. The slope term is the receiver's own depth
    // GRADIENT (#1017) — the thing a scalar bias provably cannot do — so 0 ships that fix disabled
    // behind a setting no project knows to turn on.
    assert_eq!(default.shadow_bias_slope, 0.0);
}

#[test]
fn the_frame_never_asks_for_render_settings() {
    // The bug's CLASS, not its instance. `RenderSettings` is the author's asset; what a frame may
    // read is the derived struct `apply` publishes. Asking for the asset compiles, runs, returns
    // `None` forever, and takes whatever fallback the caller wrote — silently.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // This module is where the asset is turned into what the
            // frame reads, so it is the one place allowed to hold it.
            if path.ends_with("settings.rs") || path.ends_with("settings/tests.rs") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if text.contains("get::<crate::settings::RenderSettings>")
                || text.contains("get::<RenderSettings>")
            {
                offenders.push(path);
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "these read the author's asset out of Resources, where it never is: {offenders:#?}",
    );
}
