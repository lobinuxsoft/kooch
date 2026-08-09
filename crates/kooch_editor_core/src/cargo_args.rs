//! What the editor adds to every `cargo` invocation against a project.
//!
//! A project's default build is the **game** (#558): no editor, no remote
//! server, no plugin API. Everything the editor needs of a project —
//! loading its component types, driving its ECS over a socket, drawing
//! the physics overlay — lives behind its `editor` feature.
//!
//! So the editor has to ask for it, every time, and this is the one place
//! that knows so. Four call sites build or run a project (the launcher,
//! Play, the remote session, and packaging); four copies of a feature
//! flag is four chances for one of them to be forgotten, and the symptom
//! would be a project that opens with none of its own components — which
//! reads as a broken editor rather than a missing flag.

use std::path::Path;
use std::process::Command;

/// The project feature that turns on everything authoring needs.
///
/// Matches the `[features]` block `generate_cargo_toml` writes.
pub(crate) const AUTHORING: &str = "editor";

/// Adds the authoring feature to `cmd`.
///
/// Additive rather than `--no-default-features`: `editor` includes
/// `game`, so an authoring build is the game plus the tools, and Play
/// runs the same code a player would.
pub(crate) fn authoring(cmd: &mut Command) -> &mut Command {
    cmd.args(["--features", AUTHORING])
}

/// The name of a project's authoring binary.
///
/// A second `[[bin]]`, so the game's target does not produce it and —
/// through `required-features` — cannot.
pub(crate) fn editor_bin(crate_name: &str) -> String {
    format!("{crate_name}_editor")
}

/// The crate name `manifest` declares.
///
/// Read from the file rather than derived from the folder: a project
/// directory can be renamed on disk and the crate keeps the name it was
/// created with. Falls back to the sanitised folder name, which is what
/// the two would agree on anyway for a project nobody moved.
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
///
/// Stops at the next section header, or a `[[bin]]` name would answer for
/// the package — and the authoring binary is precisely a `name` that is
/// not the crate's.
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
