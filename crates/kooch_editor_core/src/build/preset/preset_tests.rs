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

/// A Windows build needs the suffix whatever the name field says, and a
/// Linux one must not get it.
#[test]
fn windows_gets_an_exe_suffix() {
    let windows = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
        ..Default::default()
    };
    let linux = BuildPreset::default();

    assert_eq!(windows.binary_name("demo"), "demo.exe");
    assert_eq!(linux.binary_name("demo"), "demo");
}

#[test]
fn an_explicit_name_wins_over_the_crate() {
    let preset = BuildPreset {
        executable_name: "  My Game  ".to_owned(),
        ..Default::default()
    };

    assert_eq!(preset.binary_name("demo"), "My Game");
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

#[test]
fn the_profile_directory_follows_the_flag() {
    assert_eq!(BuildPreset::default().profile_dir(), "release");
    assert_eq!(
        BuildPreset {
            release: false,
            ..Default::default()
        }
        .profile_dir(),
        "debug",
    );
}

/// What "make a build" means before anyone has opinions: this machine,
/// optimised, packed.
#[test]
fn the_default_is_a_shippable_build() {
    let preset = BuildPreset::default();

    assert!(preset.release, "the default build is not optimised");
    assert!(preset.pack_assets, "the default build ships loose assets");
    assert!(preset.is_host());
}

#[test]
fn a_preset_round_trips_through_ron() {
    let preset = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
        output_dir: "dist".to_owned(),
        executable_name: "game".to_owned(),
        release: false,
        features: "cheats".to_owned(),
        pack_assets: false,
        runnable: true,
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
    assert!(sparse.release);
    assert!(sparse.pack_assets);
}
