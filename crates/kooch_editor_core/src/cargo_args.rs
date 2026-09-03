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
    // The speed flags ride with the feature flag for the reason named at
    // the top of this file: four call sites build or run a project, and
    // four copies of anything is four chances to forget one.
    fast_link(cmd);
    cmd.args(["--features", AUTHORING])
}

/// Adds the flags that make a rebuild fast.
///
/// # 🔴 Measured, because the guess was wrong
///
/// One-line change to `roll-a-ball`, warm cache, `--bin <crate>_editor`:
///
/// | | |
/// |---|---|
/// | as it was | **14.6 s** |
/// | `mold` alone | 12.5 s |
/// | split debuginfo alone | 11.7 s |
/// | **both** | **5.9 s** |
///
/// Neither is worth much alone — 14 % and 20 % — and together they are
/// **2.5x**. They interact: a fast linker is not fast while it is still
/// copying 600 MB of DWARF into the output, and not copying it does not
/// help while the linker itself is the slow part. The binary drops from
/// 635 MB to 302 MB, which is the same fact from the other side.
///
/// # ⚠️ The first build after this changes is a FULL one
///
/// `RUSTFLAGS` and the profile are part of cargo's fingerprint, so
/// turning these on rebuilds every dependency once — measured at 85-92 s
/// here. Every build after it is the fast one.
///
/// # Why here and not in a `.cargo/config.toml`
///
/// A committed config would force `mold` on every machine that builds a
/// project, and `mold` is **optional** — a machine without it would stop
/// building entirely. This asks whether it is there.
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
///
/// Asked once. Installing it ends in a reboot, so the answer cannot
/// change while the editor runs — the same reasoning `preflight::Report`
/// uses for detecting once at startup.
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
