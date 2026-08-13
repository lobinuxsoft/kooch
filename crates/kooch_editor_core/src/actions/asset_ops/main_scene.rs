//! Which scene the project opens with (#808).
//!
//! # Why this was worth a file
//!
//! `main_scene` sat in the manifest as a field **nothing read**: the
//! runtime opened `assets/scenes/default.scene` whatever it said, and the
//! editor's own loader joined it against the project root — so a project
//! carrying the short form (`scenes/x.scene`, which several do) resolved
//! to a path that did not exist and the load was skipped in silence.
//!
//! Setting it from the asset browser is the half a person sees. This is
//! the half that has to be right: a path stored relative to the project
//! root, with `assets/` in it, or the game opens nothing and says
//! nothing.

use std::path::{Path, PathBuf};

use kooch_core::resource::Resources;

use crate::project_state::ProjectState;

/// Points the open project's manifest at `path` and saves it.
pub(super) fn set_main_scene(resources: &mut Resources, path: &Path) {
    let Some(state) = resources.get_mut::<ProjectState>() else {
        return;
    };
    let Some(project) = state.active_project.as_mut() else {
        tracing::warn!("no project open — nothing to set a main scene on");
        return;
    };
    let Some(relative) = relative_to_root(&project.root_path, path) else {
        // A scene outside the project cannot be its starting scene: the
        // manifest travels with the project and a path pointing out of it
        // resolves to nothing on any other machine.
        tracing::warn!(
            scene = %path.display(),
            root = %project.root_path.display(),
            "that scene is not inside this project",
        );
        return;
    };
    if project.manifest.main_scene.as_deref() == Some(relative.as_str()) {
        return;
    }
    project.manifest.main_scene = Some(relative.clone());
    let root = project.root_path.clone();
    match project.manifest.save(&root) {
        Ok(()) => tracing::info!(scene = %relative, "main scene set"),
        // 🔴 Rolled back rather than left as it is. The panel reads the
        // manifest in memory to draw the mark, so a failed write would
        // put a badge on a scene the built game will not open — a lie
        // that survives until someone ships.
        Err(e) => {
            project.manifest.main_scene = None;
            tracing::error!("failed to write the manifest: {e}");
        }
    }
}

/// `path` as the manifest stores it: relative to the project root, with
/// forward slashes.
///
/// 🔴 Forward slashes on every platform. The manifest is a project file
/// that travels between machines, and `assets\scenes\x.scene` written on
/// Windows resolves to a single filename on Linux — one that does not
/// exist, failing the `.exists()` guard, loading nothing, reporting
/// nothing.
pub(super) fn relative_to_root(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let text = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    (!text.is_empty()).then_some(text)
}

/// The main scene as an absolute path, for whoever is drawing the tree.
///
/// `None` when no project is open or its manifest names nothing — both
/// mean "no scene is marked", which is what the browser needs to know.
///
/// Normalised on the way out, so a project carrying the short form is
/// marked on the file that will actually open rather than on nothing.
pub(crate) fn main_scene_path(state: Option<&ProjectState>) -> Option<PathBuf> {
    let project = state?.active_project.as_ref()?;
    let named = project.manifest.main_scene.as_deref()?;
    Some(
        project
            .root_path
            .join(kooch_core::scene_paths::normalise_main_scene(named)),
    )
}

#[cfg(test)]
mod main_scene_tests;
