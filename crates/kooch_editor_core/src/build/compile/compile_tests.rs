//! #758 — what cargo is actually asked to do.

use super::*;
use crate::build::preset::{MODE_PROFILING, MODE_RELEASE};

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

/// Every mode is a release build — that is what having no debug mode
/// means. A profiling build compiled without optimisations would report
/// a frame time several times too large, against a 13.9 ms budget.
#[test]
fn every_mode_asks_for_release() {
    for mode in [MODE_RELEASE, MODE_PROFILING] {
        let preset = BuildPreset {
            mode,
            ..Default::default()
        };
        assert!(
            args(&preset).iter().any(|a| a == "--release"),
            "mode {mode} did not ask for --release",
        );
    }
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

    assert!(
        !command
            .get_envs()
            .any(|(key, _)| key.to_string_lossy().starts_with("CFLAGS_")),
        "a host build picked up a cross-compilation CFLAGS",
    );
}

/// 🔴 Through the environment, never the project's `Cargo.toml`: that
/// manifest is written once, when the project is created, so a
/// `[profile.release]` in the template would reach new projects and skip
/// every existing one without saying so.
#[test]
fn a_build_is_optimised_all_the_way() {
    let command = cargo_command(&BuildPreset::default(), Path::new("/proj"), "demo");
    let envs: Vec<(String, String)> = command
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().into_owned(),
                v?.to_string_lossy().into_owned(),
            ))
        })
        .collect();

    assert!(
        envs.contains(&("CARGO_PROFILE_RELEASE_LTO".to_owned(), "fat".to_owned())),
        "no link-time optimisation: {envs:?}",
    );
    assert!(
        envs.contains(&(
            "CARGO_PROFILE_RELEASE_CODEGEN_UNITS".to_owned(),
            "1".to_owned(),
        )),
        "the optimiser still sees the crate in pieces: {envs:?}",
    );
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

    let measured = BuildPreset {
        mode: MODE_PROFILING,
        ..Default::default()
    };
    assert_eq!(
        built_binary(&measured, Path::new("/proj"), "demo"),
        Path::new("/proj/target/release/demo"),
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

/// A preset with a glibc floor goes through zigbuild, and the version
/// rides on the target rather than being passed separately — that is the
/// only spelling zigbuild reads.
#[test]
fn a_glibc_floor_goes_through_zigbuild() {
    let preset = BuildPreset {
        target_triple: "x86_64-unknown-linux-gnu".to_owned(),
        min_glibc: "2.28".to_owned(),
        ..Default::default()
    };

    let args = args(&preset);
    assert_eq!(args[0], "zigbuild");
    let at = args.iter().position(|a| a == "--target").unwrap();
    assert_eq!(args[at + 1], "x86_64-unknown-linux-gnu.2.28");
}

/// 🔴 Without a `--target` zigbuild has nothing to attach the version to
/// and the floor is silently ignored — a build that looks like it worked
/// and still will not start on the handheld. So a host preset gains one.
#[test]
#[cfg(target_os = "linux")]
fn a_floor_gives_a_host_build_a_target() {
    let preset = BuildPreset {
        min_glibc: "2.28".to_owned(),
        ..Default::default()
    };

    let args = args(&preset);
    let at = args.iter().position(|a| a == "--target").unwrap();
    assert!(args[at + 1].ends_with(".2.28"), "{}", args[at + 1]);
    // And cargo puts anything with a target under its own folder, so the
    // packager has to look there rather than in `target/release`.
    let built = built_binary(&preset, Path::new("/proj"), "demo");
    let built = built.to_string_lossy();
    assert!(
        built.starts_with("/proj/target/") && built.ends_with("/release/demo"),
        "{built}",
    );
    assert_ne!(built, "/proj/target/release/demo");
}

/// The floor is a glibc version, so it means nothing for a target that
/// does not have one — and passing `x86_64-pc-windows-gnu.2.28` fails.
#[test]
fn a_floor_is_ignored_off_linux() {
    let preset = BuildPreset {
        target_triple: "x86_64-pc-windows-gnu".to_owned(),
        min_glibc: "2.28".to_owned(),
        ..Default::default()
    };

    let args = args(&preset);
    assert_eq!(args[0], "build");
    let at = args.iter().position(|a| a == "--target").unwrap();
    assert_eq!(args[at + 1], "x86_64-pc-windows-gnu");
}

/// 🔴 The link fails on symbols of the *build machine's* libasound, which
/// is not the library the game loads. Only when a floor was asked for, and
/// appended so a project's own flags survive.
#[test]
fn a_floor_allows_undefined_host_symbols() {
    let flags = |preset: &BuildPreset| -> Option<String> {
        cargo_command(preset, Path::new("/proj"), "demo")
            .get_envs()
            .find(|(k, _)| *k == "RUSTFLAGS")
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    };

    let floored = BuildPreset {
        target_triple: "x86_64-unknown-linux-gnu".to_owned(),
        min_glibc: "2.28".to_owned(),
        ..Default::default()
    };
    assert!(flags(&floored).unwrap().contains("--allow-shlib-undefined"));
    assert!(flags(&BuildPreset::default()).is_none());
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

/// 🔴 The migration leaves an edited `main.rs` alone and warns when the
/// project opens — a hundred lines above the error, in a different panel,
/// at a different time. Someone pressing Build sees a compiler error
/// naming `kooch_editor_core`, which they never wrote. So the failure
/// says it too, where it is being read.
#[test]
fn a_failed_build_explains_an_unmigrated_main() {
    let dir = std::env::temp_dir().join("kooch_unmigrated");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src/main.rs"),
        "fn main() { kooch::kooch_editor_core::run_editor_with(x); }",
    )
    .unwrap();

    let hint = unmigrated_main(&dir).expect("the cause is recognised");
    assert!(
        hint.contains("src/editor.rs"),
        "the hint does not say where it went"
    );
    assert!(hint.contains("main.rs"));
}

/// A project already game-first gets no hint, so the message does not
/// blame a file that is fine.
#[test]
fn a_migrated_main_is_not_blamed() {
    let dir = std::env::temp_dir().join("kooch_migrated");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("src/main.rs"), "fn main() { App::new().run(); }").unwrap();

    assert!(unmigrated_main(&dir).is_none());
    // And a project with no main.rs at all is not a crash.
    assert!(unmigrated_main(std::path::Path::new("/nonexistent")).is_none());
}
