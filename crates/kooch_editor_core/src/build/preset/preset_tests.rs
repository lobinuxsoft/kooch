use super::*;

/// 🔴 The one feature that would put the authoring surface back into a
/// shipped game (#558). A preset is a text field somebody can type
/// anything into, so it is dropped here rather than trusted.
#[test]
fn the_editor_feature_is_never_passed_on() {
    let preset = BuildPreset {
        features: "cheats, editor, demo".to_owned(),
        ..Default::default()
    };

    assert_eq!(preset.feature_list(), vec!["cheats", "demo"]);
}

#[test]
fn features_are_split_and_trimmed() {
    let preset = BuildPreset {
        features: " a ,, b,c , ".to_owned(),
        ..Default::default()
    };

    assert_eq!(preset.feature_list(), vec!["a", "b", "c"]);
}

#[test]
fn no_features_is_an_empty_list() {
    assert!(BuildPreset::default().feature_list().is_empty());
}

/// The mode is what puts the profiler in the build, so a preset left on
/// Release produces a game with no listening socket.
#[test]
fn the_profiler_is_opt_in() {
    assert!(!BuildPreset::default().is_profiling());

    let measured = BuildPreset {
        mode: MODE_PROFILING,
        ..Default::default()
    };
    assert_eq!(measured.feature_list(), vec!["kooch/profiling"]);
}

/// Typing it as well as picking it asks cargo for the same feature
/// twice. Worse is the other order: a preset whose dropdown reads
/// Release while the build opens a socket.
#[test]
fn the_mode_decides_not_the_text() {
    let typed = BuildPreset {
        features: "profiling, cheats".to_owned(),
        mode: MODE_RELEASE,
        ..Default::default()
    };
    assert_eq!(typed.feature_list(), vec!["cheats"]);

    let both = BuildPreset {
        features: "profiling, kooch/profiling, cheats".to_owned(),
        mode: MODE_PROFILING,
        ..Default::default()
    };
    assert_eq!(both.feature_list(), vec!["cheats", "kooch/profiling"]);
}

/// Windows gets `.exe`; Linux gets its architecture, the way Unity and
/// Godot name their exports. A folder holding both is unambiguous.
#[test]
fn each_platform_gets_its_extension() {
    let preset = BuildPreset::default();
    assert_eq!(preset.binary_name("demo", Platform::Windows), "demo.exe",);
    assert_eq!(preset.binary_name("demo", Platform::Linux), "demo.x86_64",);
}

/// The platforms a preset builds, in the order they are built.
#[test]
fn a_preset_lists_the_platforms_it_builds() {
    let both = BuildPreset {
        linux: true,
        windows: true,
        ..Default::default()
    };
    assert_eq!(both.targets(), vec![Platform::Linux, Platform::Windows]);

    let neither = BuildPreset {
        linux: false,
        windows: false,
        ..Default::default()
    };
    assert!(neither.targets().is_empty());
}

/// Each platform lands in its own folder under the preset's output.
#[test]
fn a_platform_lands_under_the_output_dir() {
    let preset = BuildPreset {
        output_dir: "dist".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        preset.platform_dir(Platform::Windows),
        std::path::Path::new("dist/windows"),
    );
}

/// 🔴 A floor is a glibc version, and Windows has none.
///
/// One preset can build both, and `cargo zigbuild` rejects
/// `x86_64-pc-windows-gnu.2.28` — so the floor must reach the Linux half
/// only.
#[test]
fn a_floor_reaches_linux_alone() {
    let preset = BuildPreset {
        linux: true,
        windows: true,
        min_glibc: "2.28".to_owned(),
        ..Default::default()
    };

    assert_eq!(preset.glibc_floor(Platform::Linux), Some("2.28"));
    assert_eq!(preset.glibc_floor(Platform::Windows), None);
    assert!(preset.needs_zig());

    // And a preset that only builds Windows needs no zig at all, so the
    // check must not demand it be installed.
    let windows = BuildPreset {
        linux: false,
        windows: true,
        min_glibc: "2.28".to_owned(),
        ..Default::default()
    };
    assert!(!windows.needs_zig());
}

#[test]
fn an_explicit_name_wins_over_the_crate() {
    let preset = BuildPreset {
        executable_name: "  My Game  ".to_owned(),
        ..Default::default()
    };

    assert_eq!(
        preset.binary_name("demo", Platform::Linux),
        "My Game.x86_64"
    );
}

/// Both modes are optimised, so both land in `target/release`. The
/// profiler is a feature, not a cargo profile.
#[test]
fn every_mode_builds_release() {
    assert_eq!(BuildPreset::default().profile_dir(), "release");
    assert_eq!(
        BuildPreset {
            mode: MODE_PROFILING,
            ..Default::default()
        }
        .profile_dir(),
        "release",
    );
}

/// What "make a build" means before anyone has opinions: this machine,
/// optimised, packed.
#[test]
fn the_default_is_a_shippable_build() {
    let preset = BuildPreset::default();

    assert!(
        !preset.is_profiling(),
        "the default build opens a listening socket"
    );
    assert!(preset.pack_assets, "the default build ships loose assets");
    assert_eq!(
        preset.targets(),
        Platform::host().into_iter().collect::<Vec<_>>(),
        "the default build is not for the machine in front of the author",
    );
}

#[test]
fn a_preset_round_trips_through_ron() {
    let preset = BuildPreset {
        linux: true,
        windows: true,
        output_dir: "dist".to_owned(),
        executable_name: "game".to_owned(),
        mode: MODE_PROFILING,
        features: "cheats".to_owned(),
        pack_assets: false,
        min_glibc: "2.28".to_owned(),
    };

    let text = to_ron(&preset).unwrap();
    assert_eq!(ron::from_str::<BuildPreset>(&text).unwrap(), preset);
}

/// A preset written by an older editor has to keep loading and pick up
/// the new fields' defaults, or an engine update breaks every project's
/// build configuration.
#[test]
fn an_older_preset_still_loads() {
    let sparse: BuildPreset = ron::from_str("(output_dir: \"dist\")").unwrap();

    assert_eq!(sparse.output_dir, "dist");
    assert!(sparse.pack_assets);
    // 🔴 And a preset that predates the field does not acquire a
    // listening socket by being loaded by a newer editor (#558).
    assert!(!sparse.is_profiling());
}

/// 🔴 The migration that matters. `mode` defaults to Release, so a
/// preset written before the dropdown — the one saying `profiling: true`
/// — would otherwise load as Release and build a game with no
/// instrumentation in it. Nothing would fail: the build succeeds, the
/// panel offers to connect, and the connection times out against a game
/// that never opened the port.
#[test]
fn a_preset_keeps_its_profiler() {
    let legacy = "(release: true, profiling: true, runnable: false, min_glibc: \"2.28\")";
    let preset = load(legacy);

    assert!(
        preset.is_profiling(),
        "a preset that asked to be measured lost its profiler",
    );
    assert_eq!(preset.feature_list(), vec!["kooch/profiling"]);
    // And the fields it shares with the new shape survive the trip.
    assert_eq!(preset.min_glibc, "2.28");
}

/// The other direction: a preset that shipped must not acquire a
/// listening socket by being opened in a newer editor (#558).
#[test]
fn a_shipping_preset_stays_shipping() {
    assert!(!load("(release: true, profiling: false, runnable: true)").is_profiling());
    // `runnable` is gone entirely, and an unknown field is not an error.
    assert!(!load("(runnable: true)").is_profiling());
}

/// There is no debug mode any more, so the presets that used it have to
/// land somewhere. They land on Profiling — a debug build was something
/// you ran and looked at — and the loader says so rather than quietly
/// building the other thing.
#[test]
fn a_debug_preset_becomes_profiling() {
    assert!(load("(release: false)").is_profiling());
}

/// Reads a preset the way the asset loader does, migration included.
fn load(text: &str) -> BuildPreset {
    let mut ctx = LoadContext::new(std::path::Path::new("Development.buildpreset"));
    BuildPresetLoader
        .load(text.as_bytes(), &mut ctx)
        .expect("a preset the editor wrote has to load")
}

/// 🔴 The migration that matters now: a preset written before the
/// toggles carries its platform only in `target_triple`, a field the
/// struct no longer has.
///
/// Serde drops unknown fields without a word, so without reading it back
/// deliberately every existing preset would open with nothing ticked,
/// build nothing, and have that emptiness written over it on the first
/// save.
#[test]
fn an_old_presets_triple_becomes_a_toggle() {
    let windows: BuildPreset = load(r#"(target_triple: "x86_64-pc-windows-gnu")"#);
    assert!(windows.windows, "the Windows toggle did not come on");
    assert!(!windows.linux, "it gained a platform it never asked for");

    let linux: BuildPreset = load(r#"(target_triple: "x86_64-unknown-linux-gnu")"#);
    assert!(linux.linux);
    assert!(!linux.windows);
}

/// An empty triple meant "this machine", which is the host — not a
/// triple that failed to parse.
#[test]
fn an_empty_triple_becomes_the_host() {
    let preset: BuildPreset = load(r#"(target_triple: "", output_dir: "dist")"#);

    assert_eq!(
        preset.targets(),
        Platform::host().into_iter().collect::<Vec<_>>(),
    );
}

/// ⚠️ A preset written *after* the toggles must not be migrated: its
/// file has no `target_triple`, and reading an absent field as "the
/// host" would turn a deliberately-unticked Linux box back on.
#[test]
fn a_new_preset_is_not_migrated() {
    let preset: BuildPreset = load(r#"(linux: false, windows: true)"#);

    assert!(!preset.linux, "an absent triple switched Linux back on");
    assert!(preset.windows);
}
