//! Dependencies build optimised in a project's dev build.
//!
//! 🔴 Unoptimised, the engine, the ECS and Rapier run 10–50× slower, and the remote host the editor
//! drives is a dev build. The project crate stays at `opt-level = 0`, so the code being iterated
//! on still recompiles fast; only the dependencies are optimised, once, and cached.

use std::fs;
use std::path::Path;

use super::ProjectError;

/// Appended to a project's `Cargo.toml`. The header is also what marks it as present.
pub(super) const DEV_PROFILE: &str = r#"
# Dependencies — the engine included — are optimised even in a dev build: unoptimised, the ECS
# and physics run an order of magnitude slower. Your own crate stays unoptimised, so it still
# recompiles fast.
[profile.dev.package."*"]
opt-level = 3
"#;

const HEADER: &str = "[profile.dev.package.\"*\"]";

/// Adds [`DEV_PROFILE`] to a manifest that has no rule for its dependencies yet. `true` when it
/// wrote; a manifest that already says something about them is left as its author wrote it.
pub(crate) fn add_dev_profile(project_root: &Path) -> Result<bool, ProjectError> {
    let path = project_root.join("Cargo.toml");
    let text = fs::read_to_string(&path).map_err(ProjectError::Io)?;
    match with_dev_profile(&text) {
        Some(out) => {
            fs::write(&path, out).map_err(ProjectError::Io)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn with_dev_profile(text: &str) -> Option<String> {
    if text.lines().any(|line| line.trim() == HEADER) {
        return None;
    }
    Some(format!("{}\n{}", text.trim_end(), DEV_PROFILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_manifest_gains_it() {
        let out = with_dev_profile("[package]\nname = \"x\"\n").expect("adds");
        assert!(out.contains(HEADER));
        assert!(out.contains("opt-level = 3"));
    }

    /// Adding twice would be a duplicate-key error that stops the build.
    #[test]
    fn an_existing_profile_is_kept() {
        let once = with_dev_profile("[package]\n").unwrap();
        assert_eq!(with_dev_profile(&once), None);
    }
}
