//! What the editor adds to every `cargo` invocation against a project.

use std::path::Path;
use std::process::Command;

/// The project feature that turns on everything authoring needs.
///
/// Matches the `[features]` block `generate_cargo_toml` writes.
pub(crate) const AUTHORING: &str = "editor";

/// Adds the authoring feature to `cmd`.
pub(crate) fn authoring(cmd: &mut Command) -> &mut Command {
    // The speed flags ride with the feature flag for the reason named at
    // the top of this file: four call sites build or run a project, and
    // four copies of anything is four chances to forget one.
    fast_link(cmd);
    cmd.args(["--features", AUTHORING])
}

/// Adds the flags that make a rebuild fast.
pub(crate) fn fast_link(cmd: &mut Command) -> &mut Command {
    // Unconditional: it needs nothing installed, and it is the half that
    // shrinks the output rather than the half that links it faster.
    cmd.env("CARGO_PROFILE_DEV_SPLIT_DEBUGINFO", "unpacked");

    if !has_mold() {
        return cmd;
    }
    // Appended, never replaced: a `RUSTFLAGS` already in the environment
    // is somebody's deliberate choice, and overwriting it silently is
    // how a build stops doing what its author asked.
    let existing = std::env::var("RUSTFLAGS").unwrap_or_default();
    let flag = "-C link-arg=-fuse-ld=mold";
    let flags = match existing.is_empty() {
        true => flag.to_owned(),
        false if existing.contains("fuse-ld") => existing,
        false => format!("{existing} {flag}"),
    };
    cmd.env("RUSTFLAGS", flags)
}

/// Whether `mold` is on this machine.
fn has_mold() -> bool {
    static PRESENT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *PRESENT.get_or_init(|| {
        let found = Command::new("mold")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        match found {
            true => tracing::info!("linking project builds with mold"),
            false => tracing::info!(
                "mold is not installed — project builds link with the system linker, \
                 which measured 2.5x slower here",
            ),
        }
        found
    })
}

/// The name of a project's authoring binary.
pub(crate) fn editor_bin(crate_name: &str) -> String {
    format!("{crate_name}_editor")
}

/// The crate name `manifest` declares.
pub(crate) fn crate_name(manifest: &Path) -> String {
    if let Ok(text) = std::fs::read_to_string(manifest)
        && let Some(name) = declared_name(&text)
    {
        return name;
    }
    manifest
        .parent()
        .and_then(|dir| dir.file_name())
        .map(|n| crate::project::sanitize_crate_name(&n.to_string_lossy()))
        .unwrap_or_else(|| "project".to_owned())
}

/// The value of the first `name = "…"` under `[package]`.
fn declared_name(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package && let Some(value) = line.strip_prefix("name") {
            let value = value.trim_start().strip_prefix('=')?.trim();
            return Some(value.trim_matches('"').to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests;
