use super::{PROJECT_GITIGNORE, PROJECT_MANIFEST_FILE};

/// The reason the file exists: a debug build of a project linking this
/// engine is gigabytes.
#[test]
fn build_output_is_ignored() {
    assert!(PROJECT_GITIGNORE.lines().any(|line| line == "/target"));
}

/// Ignoring either of these breaks `git clone && cargo run`, which is
/// the one thing a project's repository has to do.
#[test]
fn nothing_a_fresh_clone_needs_is_ignored() {
    for needed in [
        "Cargo.lock",
        "registrations.rs",
        PROJECT_MANIFEST_FILE,
        "scenes",
        "assets",
    ] {
        assert!(
            !PROJECT_GITIGNORE.contains(needed),
            "{needed} is required to build or open the project; ignoring it breaks a clone",
        );
    }
}
