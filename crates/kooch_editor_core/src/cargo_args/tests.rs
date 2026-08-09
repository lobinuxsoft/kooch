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
