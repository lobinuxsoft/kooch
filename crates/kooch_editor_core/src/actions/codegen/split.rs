//! Bringing an existing project to a game-first shape (#558).
//!
//! Projects scaffolded before this build the editor by default: `main.rs`
//! matches on `--game` and falls through to `run_editor_with`, and the
//! manifest asks for `kooch`'s `editor` and `remote` features
//! unconditionally. A release binary of one of those opens the **editor**
//! when double-clicked, and carries the whole authoring surface whether
//! or not anything can reach it.
//!
//! The editor owns the scaffold's shape — it wrote it — so it migrates on
//! open, the way [`migrate_to_library`](super::migrate_to_library) does.
//!
//! # What this will not do
//!
//! **Rewrite a `main.rs` somebody edited.** The mode dispatcher was
//! generated, but the arms are a place to put code, and a migration that
//! silently deleted a line of gameplay would be worse than one that did
//! nothing. A `main.rs` that no longer matches what the editor generated
//! is left alone with a warning naming the two files.

use std::path::Path;

use crate::project::{generate_editor_rs, sanitize_crate_name};

/// The features block the manifest gains, and the dependency line it
/// loses the editor from.
const FEATURES: &str = r#"[features]
default = ["game"]
game = ["kooch/physics", "kooch/gravity", "kooch/camera", "kooch/audio"]
editor = [
    "game",
    "kooch/editor",
    "kooch/remote",
    "kooch/dynamic",
    "kooch/physics-debug-render",
]

"#;

/// Splits authoring out of a project's game build.
///
/// Each step is skipped when already applied, so this is a no-op on a
/// project created after #558 and on one already migrated.
pub(crate) fn split_authoring(project_root: &Path, crate_name: &str) {
    let crate_name = sanitize_crate_name(crate_name);
    let manifest_path = project_root.join("Cargo.toml");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        return;
    };
    // Somebody else's manifest: leave it be. Same rule as the sibling
    // migrations.
    if !manifest.contains("kooch = {") {
        return;
    }

    if let Some(updated) = migrate_manifest(&manifest, &crate_name) {
        match std::fs::write(&manifest_path, updated) {
            Ok(()) => tracing::info!(
                "Cargo.toml: the game is the default build; authoring moved behind \
                 the `editor` feature (#558)",
            ),
            Err(e) => {
                tracing::error!(file = %manifest_path.display(), error = %e, "failed to split the manifest");
                return;
            }
        }
    }

    let editor_rs = project_root.join("src").join("editor.rs");
    if !editor_rs.exists()
        && let Err(e) = std::fs::write(&editor_rs, generate_editor_rs(&crate_name))
    {
        tracing::error!(file = %editor_rs.display(), error = %e, "failed to write editor.rs");
        return;
    }

    migrate_main(project_root, &crate_name);
    migrate_lib(project_root);
}

/// The manifest with a `[features]` block, a gated authoring binary, and
/// a `kooch` line that no longer asks for the editor. `None` when it
/// already has all three.
fn migrate_manifest(manifest: &str, crate_name: &str) -> Option<String> {
    let mut out = manifest.to_owned();
    let mut changed = false;

    if !out.contains("[features]") {
        // Before `[lib]` if there is one, else before `[dependencies]`,
        // so the file still reads top-down.
        let at = out
            .find("[lib]")
            .or_else(|| out.find("[[bin]]"))
            .or_else(|| out.find("[dependencies]"))
            .unwrap_or(out.len());
        out.insert_str(at, FEATURES);
        changed = true;
    }

    let authoring = crate::cargo_args::editor_bin(crate_name);
    if !out.contains(&format!("name = \"{authoring}\"")) {
        let bin = format!(
            "\n[[bin]]\nname = \"{authoring}\"\npath = \"src/editor.rs\"\nrequired-features = [\"editor\"]\n",
        );
        let at = out.find("[dependencies]").unwrap_or(out.len());
        out.insert_str(at, bin.trim_start_matches('\n'));
        changed = true;
    }

    // 🔴 The line that made a release binary carry the editor. Rewritten
    // to the bare path: what a project needs now comes from its own
    // `[features]`, which is what makes a game build a game build.
    if let Some(line) = kooch_line(&out)
        && line.contains("features")
    {
        let path = line
            .find("path = \"")
            .map(|at| at + "path = \"".len())
            .and_then(|start| line[start..].find('"').map(|end| &line[start..start + end]));
        if let Some(path) = path {
            out = out.replace(&line, &format!("kooch = {{ path = \"{path}\" }}"));
            changed = true;
        }
    }

    changed.then_some(out)
}

/// The whole `kooch = { … }` line, if the manifest has one.
fn kooch_line(manifest: &str) -> Option<String> {
    manifest
        .lines()
        .find(|line| line.trim_start().starts_with("kooch = {"))
        .map(str::to_owned)
}

/// Replaces the old mode dispatcher with a `main.rs` that is only the
/// game — but only when it is still what the editor generated.
fn migrate_main(project_root: &Path, crate_name: &str) {
    let path = project_root.join("src").join("main.rs");
    let Ok(main) = std::fs::read_to_string(&path) else {
        return;
    };
    // Already game-first, or never the old shape.
    if !main.contains("run_editor_with") {
        return;
    }

    if main != old_scaffold(crate_name) {
        // 🔴 Not touched, and said loudly. The build still works — the
        // authoring binary is what the editor runs now — but this file
        // still opens the editor, so a release of it is not shippable
        // and nothing else would ever mention that.
        tracing::warn!(
            file = %path.display(),
            "src/main.rs was edited, so it was left as it is — it still starts the \
             editor when the game is run. Copy your gameplay setup into the plain \
             `App::new()` form (see src/editor.rs for what moved) so a release \
             build is the game only (#558)",
        );
        return;
    }

    match std::fs::write(&path, crate::project::generate_main_rs(crate_name)) {
        Ok(()) => tracing::info!("src/main.rs: is now the game, with no editor in it"),
        Err(e) => tracing::error!(file = %path.display(), error = %e, "failed to rewrite main.rs"),
    }
}

/// Puts the plugin export behind the `editor` feature: a game links no
/// plugin API, so an ungated `lib.rs` no longer compiles in a game build.
fn migrate_lib(project_root: &Path) {
    let path = project_root.join("src").join("lib.rs");
    let Ok(lib) = std::fs::read_to_string(&path) else {
        return;
    };
    if lib.contains("#[cfg(feature = \"editor\")]") || !lib.contains("export_plugin!") {
        return;
    }
    // Regenerated rather than patched: this file is editor-managed and
    // says so in its own first line, and the gated form nests the whole
    // block in a module — which is not an edit, it is the file.
    let crate_name = crate::cargo_args::crate_name(&project_root.join("Cargo.toml"));
    match std::fs::write(&path, crate::project::generate_lib_rs(&crate_name)) {
        Ok(()) => tracing::info!("src/lib.rs: the plugin export is behind the `editor` feature"),
        Err(e) => tracing::error!(file = %path.display(), error = %e, "failed to rewrite lib.rs"),
    }
}

/// The `main.rs` the editor generated before #558.
///
/// Kept verbatim so a project can be recognised as untouched. It is dead
/// weight the day no project predates the split, and until then it is the
/// difference between migrating and overwriting someone's work.
fn old_scaffold(crate_name: &str) -> String {
    OLD_MAIN.replace("PROJECT_CRATE", crate_name)
}

const OLD_MAIN: &str = r##"use kooch::prelude::*;

// The project's own library — the same code the editor loads as a dylib.
// `registrations` is editor-managed: regenerated whenever you create or
// register scripts. Do not edit it by hand.
use PROJECT_CRATE::registrations;

fn main() {
    // `cargo run`            → the editor, with your components (authoring).
    // `cargo run -- --game`  → the game (what the editor's Play button runs).
    // `cargo run -- --remote`→ headless authoring host: your components +
    //                          the remote server, driven by the standalone
    //                          editor over a local socket. Gameplay starts
    //                          paused; the
    //                          editor's Play button starts it without a
    //                          rebuild, in the editor's own viewport.
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--game") {
        // Game runtime: components + gameplay systems.
        let mut app = App::new();
        app.add_plugins(DefaultPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: true });
        app.run();
    } else if args.iter().any(|a| a == "--remote") {
        // Remote authoring host: components register (so the editor's
        // Inspector sees them) and systems register paused — the editor
        // toggles `Playing` over the wire to run them in place.
        // Headless on purpose: the editor draws this world in its own
        // viewport, so a window here would show the same scene twice.
        let mut app = App::new();
        app.add_plugins(RemoteHostPlugins);
        app.add_plugin(registrations::ProjectRegistrations { run_systems: false });
        app.add_plugin(kooch::kooch_remote::RemotePlugin::new());
        app.run();
    } else {
        // Editor embedded in the project: register components (for the
        // Inspector) but do NOT run gameplay systems.
        kooch::kooch_editor_core::run_editor_with(registrations::ProjectRegistrations {
            run_systems: false,
        });
    }
}
"##;

#[cfg(test)]
mod tests;
