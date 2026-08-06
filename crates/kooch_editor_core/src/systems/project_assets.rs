//! `scan_project_assets_system` — keeps the active project's
//! `<root>/assets/` tree mirrored into the editor-wide
//! `AssetDatabase`.
//!
//! Runs every frame at `Stage::PreUpdate`. Compares the current
//! `ProjectState.active_project` path against
//! [`LastScannedProject`]; if it differs, scans
//! `<project>/assets/` into the database and triggers the typed
//! eager-import pass so newly-found `.glb` / `.kooch_material.ron`
//! / `.png` files appear in the inspector picker tagged
//! `[project]` at the next frame.
//!
//! Project closure (active_project → None) does NOT evict
//! database entries: a GUID assigned by the previous project is
//! still valid; closing a project just means the editor cannot
//! resolve those bytes anymore. Eviction lands when an actual
//! workflow demands it.

use std::path::PathBuf;

use kooch_core::asset_database::AssetDatabase;
use kooch_core::resource::Resources;
use kooch_render::plugin::assets::eager_import_with;

use crate::project_state::ProjectState;

/// Tracks the project root the asset database last scanned, so the
/// system can detect open / change events and avoid re-scanning
/// every frame.
#[derive(Debug, Default, Clone)]
pub struct LastScannedProject {
    pub root: Option<PathBuf>,
}

/// PreUpdate system: react to project open / close by re-scanning
/// the project's `assets/` tree and eager-importing its typed
/// payload.
pub fn scan_project_assets_system(resources: &mut Resources) {
    let current = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.active_project.as_ref().map(|ap| ap.root_path.clone()));

    let last = resources
        .get::<LastScannedProject>()
        .map(|l| l.root.clone())
        .unwrap_or(None);

    if current == last {
        return;
    }

    if let Some(root) = current.as_ref() {
        let assets_root = root.join("assets");
        if assets_root.exists() {
            // Run the database scan first so eager-import can see
            // every existing sidecar and skip ones it does not need
            // to re-create.
            // What this binary can load, so a `.rendersettings` or any
            // other hand-written asset registers on the first scan.
            let known = resources
                .get::<kooch_core::asset_loader::AssetServer>()
                .map(|server| server.known_extensions())
                .unwrap_or_default();
            let scanned = match resources.get_mut::<AssetDatabase>() {
                Some(db) => match db.scan_directory_adopting(&assets_root, &known) {
                    Ok(report) => Some(report),
                    Err(e) => {
                        tracing::warn!(
                            target: "kooch_editor_core::project_assets",
                            project_root = %root.display(),
                            error = %e,
                            "project asset scan failed",
                        );
                        None
                    }
                },
                None => {
                    tracing::warn!(
                        target: "kooch_editor_core::project_assets",
                        "AssetDatabase missing; skipping project asset scan",
                    );
                    None
                }
            };
            if let Some(report) = scanned {
                tracing::info!(
                    target: "kooch_editor_core::project_assets",
                    project_root = %root.display(),
                    registered = report.registered,
                    duplicates = report.duplicates,
                    orphans = report.orphans,
                    "project asset scan complete",
                );
            }
            // eager_import_with walks the project's assets/ tree and
            // loads every file with a recognised typed extension —
            // generating fresh sidecars where missing and back-filling
            // `asset_type` on legacy ones. The picker sees the
            // entries tagged [project] on the next frame.
            eager_import_with(resources, &assets_root);
        } else {
            tracing::debug!(
                target: "kooch_editor_core::project_assets",
                project_root = %root.display(),
                "project has no assets/ directory; skipping scan",
            );
        }
    } else {
        tracing::debug!(
            target: "kooch_editor_core::project_assets",
            "project closed; database entries retained",
        );
    }

    // Update the marker after the scan / open path runs so future
    // frames short-circuit until the project changes again.
    if let Some(last) = resources.get_mut::<LastScannedProject>() {
        last.root = current;
    } else {
        resources.insert(LastScannedProject { root: current });
    }
}

/// PreUpdate system: if the active project's `src/main.rs` has been
/// deleted, regenerate the entrypoint + `registrations.rs` (scanning
/// every `.rs` under `src/`). Only fires when `src/` exists and
/// `main.rs` is missing, so it never fights a user editing an existing
/// `main.rs`.
pub fn ensure_main_exists_system(resources: &mut Resources) {
    let regenerate = resources
        .get::<ProjectState>()
        .and_then(|ps| ps.active_project.as_ref())
        .map(|ap| {
            let src = ap.root_path.join("src");
            src.is_dir() && !src.join("main.rs").exists()
        })
        .unwrap_or(false);
    if regenerate {
        crate::actions::register_scripts(resources);
    }
}
