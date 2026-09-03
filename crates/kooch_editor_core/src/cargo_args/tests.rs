use super::*;

/// The feature name here and the one the scaffold writes are the same
/// string in two files. They agree, or the editor asks for a feature the
/// project does not have and cargo refuses to build anything.
#[test]
fn the_feature_matches_the_scaffold() {
    let manifest = crate::project::generate_cargo_toml_for_test("demo", "/engine");
    assert!(
        manifest.contains(&format!("\n{AUTHORING} = [")),
        "the scaffold declares no `{AUTHORING}` feature:\n{manifest}",
    );
}

/// Likewise for the binary name: the editor runs it by name, and the
/// manifest declares it.
#[test]
fn the_binary_matches_the_scaffold() {
    let manifest = crate::project::generate_cargo_toml_for_test("demo", "/engine");
    assert!(
        manifest.contains(&format!("name = \"{}\"", editor_bin("demo"))),
        "the scaffold declares no `{}` binary:\n{manifest}",
        editor_bin("demo"),
    );
}

/// 🔴 The property #558 is about. A game build must not be able to reach
/// the editor, and what makes that true is `required-features` — without
/// it a plain `cargo build` produces the authoring binary too, and the
/// feature unification that comes with it puts the editor back in.
#[test]
fn the_authoring_binary_is_gated() {
    let manifest = crate::project::generate_cargo_toml_for_test("demo", "/engine");
    let at = manifest
        .find(&format!("name = \"{}\"", editor_bin("demo")))
        .expect("the authoring binary is declared");
    assert!(
        manifest[at..].contains("required-features = [\"editor\"]"),
        "the authoring binary is not gated behind its feature",
    );
}

/// The default build is the game, or a shipped artefact is whatever the
/// last person to touch the manifest happened to leave enabled.
#[test]
fn the_default_build_is_the_game() {
    let manifest = crate::project::generate_cargo_toml_for_test("demo", "/engine");
    assert!(manifest.contains("default = [\"game\"]"));
    assert!(
        !manifest.contains("default = [\"editor\""),
        "the default build is the editor — this is #558",
    );
}

// ---- the flags that make a rebuild fast -----------------------------

use super::fast_link;

/// Reads what `fast_link` set, since `Command` does not expose it back
/// as a map.
fn env_of(cmd: &std::process::Command, key: &str) -> Option<String> {
    cmd.get_envs()
        .find_map(|(k, v)| (k == key).then(|| v.map(|v| v.to_string_lossy().into_owned())))?
}

/// The half that needs nothing installed is always applied. It is also
/// the half that shrinks the binary — 635 MB to 302 MB here — which is
/// most of why the link got faster.
#[test]
fn debuginfo_is_always_split() {
    let mut cmd = std::process::Command::new("cargo");
    fast_link(&mut cmd);

    assert_eq!(
        env_of(&cmd, "CARGO_PROFILE_DEV_SPLIT_DEBUGINFO").as_deref(),
        Some("unpacked"),
    );
}

/// 🔴 A `RUSTFLAGS` already in the environment is somebody's deliberate
/// choice. Overwriting it silently is how a build stops doing what its
/// author asked — and the symptom would be a flag that "does nothing".
#[test]
fn existing_rustflags_survive() {
    // SAFETY: single-threaded test, and the variable is restored below.
    let before = std::env::var("RUSTFLAGS").ok();
    unsafe { std::env::set_var("RUSTFLAGS", "-C target-cpu=native") };

    let mut cmd = std::process::Command::new("cargo");
    fast_link(&mut cmd);
    let flags = env_of(&cmd, "RUSTFLAGS");

    match flags {
        // With mold present the flag is appended, never replacing.
        Some(flags) => assert!(
            flags.contains("target-cpu=native"),
            "the caller's own flags were dropped: {flags}",
        ),
        // Without mold nothing is set, so the environment's own survives
        // untouched — which is the same guarantee.
        None => {}
    }

    unsafe {
        match before {
            Some(value) => std::env::set_var("RUSTFLAGS", value),
            None => std::env::remove_var("RUSTFLAGS"),
        }
    }
}

/// Every authoring build gets them, which is the whole reason this
/// module exists: four call sites, and four copies is four chances to
/// forget one.
#[test]
fn authoring_carries_them_too() {
    let mut cmd = std::process::Command::new("cargo");
    super::authoring(&mut cmd);

    assert!(env_of(&cmd, "CARGO_PROFILE_DEV_SPLIT_DEBUGINFO").is_some());
}
