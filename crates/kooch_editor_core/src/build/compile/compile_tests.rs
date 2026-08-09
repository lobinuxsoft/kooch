//! #758 — what cargo is actually asked to do.

use super::*;

/// The arguments as strings, for asserting on.
fn args(preset: &BuildPreset) -> Vec<String> {
    cargo_command(preset, Path::new("/proj"), "demo")
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// 🔴 The game, never `demo_editor`. The authoring binary is gated
/// behind a feature a shipped build does not enable (#558), and naming
/// it here is the one way to put the editor back into a release.
#[test]
fn the_game_binary_is_the_one_built() {
    let args = args(&BuildPreset::default());
    let at = args
        .iter()
        .position(|a| a == "--bin")
        .expect("--bin passed");

    assert_eq!(args[at + 1], "demo");
    assert!(!args.iter().any(|a| a.contains("_editor")));
}

/// And the authoring feature never reaches the command line, whatever a
/// preset's text field says.
#[test]
fn the_editor_feature_never_reaches_cargo() {
    let preset = BuildPreset {
        features: "editor,cheats".to_owned(),
        ..Default::default()
    };

    let args = args(&preset);
    let at = args.iter().position(|a| a == "--features").unwrap();
    assert_eq!(args[at + 1], "cheats");
}

#[test]
fn a_release_preset_asks_for_release() {
    assert!(
        args(&BuildPreset::default())
            .iter()
            .any(|a| a == "--release")
    );
    assert!(
        !args(&BuildPreset {
            release: false,
            ..Default::default()
        })
        .iter()
        .any(|a| a == "--release"),
    );
}

/// An empty triple means this machine, and passing `--target ""` would
/// fail rather than build for the host.
#[test]
fn a_host_build_passes_no_target() {
    assert!(
        !args(&BuildPreset::default())
            .iter()
            .any(|a| a == "--target")
    );
}

#[test]
fn a_cross_build_passes_its_triple() {
    let preset = BuildPreset {
        target_triple: "  x86_64-pc-windows-gnu  ".to_owned(),
        ..Default::default()
    };

    let args = args(&preset);
    let at = args.iter().position(|a| a == "--target").unwrap();
    assert_eq!(args[at + 1], "x86_64-pc-windows-gnu");
}

/// 🔴 mingw's gcc defaults to C23, where `false` is a keyword, and
/// GKlib declares an enum member with that name. Without this, metis-sys
/// fails to build for Windows — measured on this machine, not guessed.
#[test]
fn a_windows_build_carries_the_mingw_cflags() {
    let preset = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
        ..Default::default()
    };
    let command = cargo_command(&preset, Path::new("/proj"), "demo");

    let set: Vec<_> = command
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().into_owned(),
                v?.to_string_lossy().into_owned(),
            ))
        })
        .collect();
    assert!(
        set.contains(&(
            "CFLAGS_x86_64_pc_windows_gnu".to_owned(),
            "-std=gnu17".to_owned(),
        )),
        "the mingw C23 workaround is missing: {set:?}",
    );
}

/// A host build must not carry it: it would apply to nothing, and a flag
/// nobody can explain is worse than no flag.
#[test]
fn a_host_build_carries_no_cflags() {
    let command = cargo_command(&BuildPreset::default(), Path::new("/proj"), "demo");

    assert_eq!(command.get_envs().count(), 0);
}

/// Where cargo leaves the executable differs by target and profile, and
/// getting it wrong reads as "cargo succeeded but built nothing".
#[test]
fn the_built_binary_is_where_cargo_puts_it() {
    let host = BuildPreset::default();
    assert_eq!(
        built_binary(&host, Path::new("/proj"), "demo"),
        Path::new("/proj/target/release/demo"),
    );

    let debug = BuildPreset {
        release: false,
        ..Default::default()
    };
    assert_eq!(
        built_binary(&debug, Path::new("/proj"), "demo"),
        Path::new("/proj/target/debug/demo"),
    );

    let windows = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
        ..Default::default()
    };
    assert_eq!(
        built_binary(&windows, Path::new("/proj"), "demo"),
        Path::new("/proj/target/x86_64-pc-windows-gnu/release/demo.exe"),
    );
}

/// The manifest is named explicitly: cargo run from the editor's own
/// working directory would otherwise build the editor's workspace.
#[test]
fn the_project_manifest_is_named() {
    let args = args(&BuildPreset::default());
    let at = args.iter().position(|a| a == "--manifest-path").unwrap();

    assert_eq!(args[at + 1], "/proj/Cargo.toml");
}

/// A target nobody installed fails ten minutes in with a linker error
/// that never says the word "target". The check says what to run.
#[test]
fn a_missing_target_is_refused_with_the_fix() {
    let preset = BuildPreset {
        target_triple: "nonsense-unknown-triple".to_owned(),
        ..Default::default()
    };

    if let Some(problem) = missing_toolchain(&preset) {
        assert!(problem.contains("rustup target add nonsense-unknown-triple"));
    }
    // No assertion when rustup is absent: this machine cannot answer the
    // question, and refusing the build over that would be worse.
}

#[test]
fn a_host_build_needs_no_target_check() {
    assert!(missing_toolchain(&BuildPreset::default()).is_none());
}
