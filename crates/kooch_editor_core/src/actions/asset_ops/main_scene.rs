//! Which scene the project opens with (#808).

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
        // 🔴 Rolled back rather than left as it is. The panel reads the manifest in memory to draw
        // the mark, so a failed write would put a badge on a scene the built game will not open — a
        // lie that survives until someone ships.
        Err(e) => {
            project.manifest.main_scene = None;
            tracing::error!("failed to write the manifest: {e}");
        }
    }
}

/// `path` as the manifest stores it: relative to the project root, with forward slashes.
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
