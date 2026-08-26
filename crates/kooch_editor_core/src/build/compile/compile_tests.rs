//! #758 — what cargo is actually asked to do.

use super::*;
use crate::build::preset::{MODE_PROFILING, MODE_RELEASE};

/// The arguments as strings, for asserting on.
fn args(preset: &BuildPreset, platform: Platform) -> Vec<String> {
    cargo_command(preset, platform, Path::new("/proj"), "demo")
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// A preset that builds one named platform and nothing else.
fn only(platform: Platform) -> BuildPreset {
    BuildPreset {
        linux: platform == Platform::Linux,
        windows: platform == Platform::Windows,
        ..Default::default()
    }
}

/// 🔴 The game, never `demo_editor`. The authoring binary is gated
/// behind a feature a shipped build does not enable (#558), and naming
/// it here is the one way to put the editor back into a release.
#[test]
fn the_game_binary_is_the_one_built() {
    let args = args(&only(Platform::Linux), Platform::Linux);
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
        ..only(Platform::Linux)
    };

    let args = args(&preset, Platform::Linux);
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
            ..only(Platform::Linux)
        };
        assert!(
            args(&preset, Platform::Linux)
                .iter()
                .any(|a| a == "--release"),
            "mode {mode} did not ask for --release",
        );
    }
}

/// 🔴 Every build names its target, the host's included.
///
/// Passing one sometimes and not others means the packager has to guess
/// afterwards whether cargo wrote to `target/release` or
/// `target/<triple>/release`. It also has to be there for a glibc floor,
/// which zigbuild has nothing to attach to without it.
#[test]
fn every_build_names_its_target() {
    for platform in Platform::ALL {
        let args = args(&only(platform), platform);
        let at = args
            .iter()
            .position(|a| a == "--target")
            .unwrap_or_else(|| panic!("{} passed no target", platform.label()));
        assert_eq!(args[at + 1], platform.triple());
    }
}

/// 🔴 mingw's gcc defaults to C23, where `false` is a keyword, and
/// GKlib declares an enum member with that name. Without this, metis-sys
/// fails to build for Windows — measured on this machine, not guessed.
#[test]
fn a_windows_build_carries_the_mingw_cflags() {
    let command = cargo_command(
        &only(Platform::Windows),
        Platform::Windows,
        Path::new("/proj"),
        "demo",
    );

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

/// A Linux build must not carry it: it would apply to nothing, and a
/// flag nobody can explain is worse than no flag.
#[test]
fn a_linux_build_carries_no_cflags() {
    let command = cargo_command(
        &only(Platform::Linux),
        Platform::Linux,
        Path::new("/proj"),
        "demo",
    );

    assert!(
        !command
            .get_envs()
            .any(|(key, _)| key.to_string_lossy().starts_with("CFLAGS_")),
        "a Linux build picked up the Windows CFLAGS",
    );
}

/// 🔴 Through the environment, never the project's `Cargo.toml`: that
/// manifest is written once, when the project is created, so a
/// `[profile.release]` in the template would reach new projects and skip
/// every existing one without saying so.
#[test]
fn a_build_is_optimised_all_the_way() {
    let command = cargo_command(
        &only(Platform::Linux),
        Platform::Linux,
        Path::new("/proj"),
        "demo",
    );
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
    assert_eq!(
        built_binary(
            &only(Platform::Linux),
            Platform::Linux,
            Path::new("/proj"),
            "demo",
        ),
        Path::new("/proj/target/x86_64-unknown-linux-gnu/release/demo"),
    );

    let measured = BuildPreset {
        mode: MODE_PROFILING,
        ..only(Platform::Linux)
    };
    assert_eq!(
        built_binary(&measured, Platform::Linux, Path::new("/proj"), "demo"),
        Path::new("/proj/target/x86_64-unknown-linux-gnu/release/demo"),
        "profiling is a feature, not a profile — both land in release",
    );

    assert_eq!(
        built_binary(
            &only(Platform::Windows),
            Platform::Windows,
            Path::new("/proj"),
            "demo",
        ),
        Path::new("/proj/target/x86_64-pc-windows-gnu/release/demo.exe"),
    );
}

/// ⚠️ The floor rides on the `--target` **argument**, not on the folder
/// cargo creates. Looking for the binary under
/// `target/x86_64-unknown-linux-gnu.2.28/` finds nothing, which reads as
/// "cargo succeeded and built nothing".
#[test]
fn a_floor_does_not_move_the_binary() {
    let preset = BuildPreset {
        min_glibc: "2.28".to_owned(),
        ..only(Platform::Linux)
    };
    assert_eq!(
        built_binary(&preset, Platform::Linux, Path::new("/proj"), "demo"),
        Path::new("/proj/target/x86_64-unknown-linux-gnu/release/demo"),
    );
}

/// A preset with a glibc floor goes through zigbuild, and the version
/// rides on the target rather than being passed separately — that is the
/// only spelling zigbuild reads.
#[test]
fn a_glibc_floor_goes_through_zigbuild() {
    let preset = BuildPreset {
        min_glibc: "2.28".to_owned(),
        ..only(Platform::Linux)
    };

    let args = args(&preset, Platform::Linux);
    assert_eq!(args[0], "zigbuild");
    let at = args.iter().position(|a| a == "--target").unwrap();
    assert_eq!(args[at + 1], "x86_64-unknown-linux-gnu.2.28");
}

/// 🔴 One preset, two platforms, and the floor must reach exactly one of
/// them.
///
/// `cargo zigbuild` spells a floor by appending it to the triple, and
/// `x86_64-pc-windows-gnu.2.28` is not a target — so a floor that
/// followed the build onto Windows would fail it on an argument nobody
/// typed. Windows also drops back to plain `cargo build`: there is
/// nothing for zigbuild to do.
#[test]
fn a_floor_reaches_linux_and_not_windows() {
    let preset = BuildPreset {
        linux: true,
        windows: true,
        min_glibc: "2.28".to_owned(),
        ..Default::default()
    };

    let linux = args(&preset, Platform::Linux);
    assert_eq!(linux[0], "zigbuild");
    let at = linux.iter().position(|a| a == "--target").unwrap();
    assert_eq!(linux[at + 1], "x86_64-unknown-linux-gnu.2.28");

    let windows = args(&preset, Platform::Windows);
    assert_eq!(windows[0], "build");
    let at = windows.iter().position(|a| a == "--target").unwrap();
    assert_eq!(windows[at + 1], "x86_64-pc-windows-gnu");
}

/// 🔴 The link fails on symbols of the *build machine's* libasound, which
/// is not the library the game loads. Only when a floor was asked for, and
/// appended so a project's own flags survive.
#[test]
fn a_floor_allows_undefined_host_symbols() {
    let flags = |preset: &BuildPreset, platform: Platform| -> Option<String> {
        cargo_command(preset, platform, Path::new("/proj"), "demo")
            .get_envs()
            .find(|(k, _)| *k == "RUSTFLAGS")
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    };

    let floored = BuildPreset {
        min_glibc: "2.28".to_owned(),
        ..only(Platform::Linux)
    };
    assert!(
        flags(&floored, Platform::Linux)
            .unwrap()
            .contains("--allow-shlib-undefined")
    );
    assert!(flags(&only(Platform::Linux), Platform::Linux).is_none());
}

/// The manifest is named explicitly: cargo run from the editor's own
/// working directory would otherwise build the editor's workspace.
#[test]
fn the_project_manifest_is_named() {
    let args = args(&only(Platform::Linux), Platform::Linux);
    let at = args.iter().position(|a| a == "--manifest-path").unwrap();

    assert_eq!(args[at + 1], "/proj/Cargo.toml");
}

/// A target nobody installed fails ten minutes in with a linker error
/// that never says the word "target". The check says what to run.
#[test]
fn a_missing_target_is_refused_with_the_fix() {
    let Some(host) = Platform::host() else {
        return;
    };
    let cross = Platform::ALL.into_iter().find(|p| *p != host);
    let Some(cross) = cross else {
        return;
    };

    let Some(problem) = missing_toolchain(&only(cross)) else {
        // The target is installed and so is everything else it needs.
        return;
    };
    // ⚠️ The same check answers for more than one thing — a missing
    // target and a missing mingw both come back here — so the assertion
    // is on the branch this test is about, not on whatever came first.
    // Asserting unconditionally made this fail the day the mingw check
    // landed, on a machine where the target was installed all along.
    if !problem.contains("is not installed") {
        return;
    }
    assert!(
        problem.contains(&format!("rustup target add {}", cross.triple())),
        "the refusal does not say what to run: {problem}",
    );
}

/// The platform this machine runs needs no target installed: it is the
/// one rustup came with.
#[test]
fn the_host_platform_needs_no_target_check() {
    let Some(host) = Platform::host() else {
        return;
    };
    assert!(missing_toolchain(&only(host)).is_none());
}

/// 🔴 A preset with nothing ticked builds nothing, and must say so.
///
/// The tempting reading is "no platform means the host" — which would
/// make an unticked box behave exactly like a ticked one, and there
/// would be no way to express "not this one".
#[test]
fn a_preset_with_no_platform_names_none() {
    let preset = BuildPreset {
        linux: false,
        windows: false,
        ..Default::default()
    };
    assert!(preset.targets().is_empty());
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

/// 🔴 A Linux-only preset must never be refused over a Windows
/// toolchain.
///
/// This is the half that can be tested on any machine, and it is the
/// half that would ruin somebody's day: a check that asked for mingw
/// unconditionally would stop every Linux build on every machine that
/// never intends to ship for Windows.
#[test]
fn a_linux_preset_is_never_asked_for_mingw() {
    let Some(host) = Platform::host() else {
        return;
    };
    if host != Platform::Linux {
        return;
    }
    let problem = missing_toolchain(&only(Platform::Linux));
    assert!(
        problem.is_none(),
        "a Linux build was refused: {}",
        problem.unwrap_or_default(),
    );
}

/// And when the tools are genuinely absent, the refusal says which one
/// and how to install it.
///
/// 🔴 `g++`, not just `gcc`. Measured: this machine had `mingw64-gcc`
/// and no `mingw64-gcc-c++`, and the build died inside `meshopt`'s build
/// script — meshoptimizer is C++ — well after cargo had accepted the
/// target. A check that only looked for a mingw `gcc` would have said
/// yes and let it through.
#[test]
fn a_missing_mingw_names_the_tool_and_the_package() {
    let Some(problem) = missing_mingw() else {
        // The tools are installed here; there is nothing to assert and
        // uninstalling them to find out would break the next build.
        return;
    };
    assert!(
        problem.contains("x86_64-w64-mingw32-g++"),
        "the refusal does not name the missing tool: {problem}",
    );
    assert!(
        problem.contains("mingw64-gcc-c++"),
        "the refusal does not say what to install: {problem}",
    );
}
