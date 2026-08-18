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
    let windows = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
        ..Default::default()
    };
    let linux = BuildPreset {
        target_triple: "x86_64-unknown-linux-gnu".to_owned(),
        ..Default::default()
    };

    assert_eq!(windows.binary_name("demo"), "demo.exe");
    assert_eq!(linux.binary_name("demo"), "demo.x86_64");
}

/// 🔴 Read off the triple, not assumed. A build for ARM named
/// `.x86_64` is a name that lies about what it runs on.
#[test]
fn the_architecture_comes_from_the_triple() {
    let arm = BuildPreset {
        target_triple: "aarch64-unknown-linux-gnu".to_owned(),
        ..Default::default()
    };

    assert_eq!(arm.binary_name("demo"), "demo.aarch64");
}

/// An empty triple means this machine, so its own architecture answers.
#[test]
fn a_host_build_uses_this_machines_arch() {
    assert_eq!(
        BuildPreset::default().binary_name("demo"),
        format!("demo.{}", std::env::consts::ARCH),
    );
}

#[test]
fn an_explicit_name_wins_over_the_crate() {
    let preset = BuildPreset {
        executable_name: "  My Game  ".to_owned(),
        ..Default::default()
    };

    assert_eq!(
        preset.binary_name("demo"),
        format!("My Game.{}", std::env::consts::ARCH),
    );
}

#[test]
fn an_empty_triple_means_this_machine() {
    assert!(BuildPreset::default().is_host());
    assert!(
        !BuildPreset {
            target_triple: "x86_64-pc-windows-gnu".to_owned(),
            ..Default::default()
        }
        .is_host()
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
    assert!(preset.is_host());
}

/// A glibc floor only means something where there is a glibc, and
/// `x86_64-pc-windows-gnu.2.28` is a target that does not exist.
#[test]
fn a_floor_only_applies_to_gnu_linux() {
    let floored = |triple: &str| {
        BuildPreset {
            target_triple: triple.to_owned(),
            min_glibc: "2.28".to_owned(),
            ..Default::default()
        }
        .glibc_floor()
        .map(str::to_owned)
    };

    assert_eq!(floored("x86_64-unknown-linux-gnu"), Some("2.28".to_owned()));
    assert_eq!(floored("x86_64-pc-windows-gnu"), None);
    assert_eq!(floored("x86_64-unknown-linux-musl"), None);
    // And no floor asked for is no floor, whatever the target.
    assert_eq!(
        BuildPreset {
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            min_glibc: "   ".to_owned(),
            ..Default::default()
        }
        .glibc_floor(),
        None,
    );
}

#[test]
fn a_preset_round_trips_through_ron() {
    let preset = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
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
