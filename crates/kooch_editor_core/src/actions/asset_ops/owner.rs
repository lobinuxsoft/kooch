//! Whose asset a path is, for the operations that must not touch the
//! engine's.
//!
//! The boundary itself is not new — `workspace_for` has drawn it since
//! the IDE entry existed, and a comment beside it claims engine assets
//! are read-only. Nothing enforced the claim (#815).

use std::path::Path;

use kooch_core::resource::Resources;

/// Whether `path` belongs to the engine rather than the open project.
///
/// 🔴 The project is asked first, and that is not a tie-break. An editor
/// built from a project resolves its engine root to that same project
/// (`bootstrap::engine_root` walks up for the nearest `assets/`), so
/// asking the engine first would make everything in it undeletable.
pub(super) fn engine_owned(project: Option<&Path>, engine: Option<&Path>, path: &Path) -> bool {
    if project.is_some_and(|root| path.starts_with(root)) {
        return false;
    }
    engine.is_some_and(|root| path.starts_with(root))
}

/// [`engine_owned`] against the open project, saying which rule stopped
/// the caller.
///
/// The message names the reach — every project sharing the install, not
/// the one that is open — because "delete failed" sends an author
/// looking for a permission problem they do not have.
pub(super) fn refuses(resources: &Resources, path: &Path) -> bool {
    let Some(state) = resources.get::<crate::project_state::ProjectState>() else {
        return false;
    };
    let project = state.active_project.as_ref().map(|p| p.root_path.as_path());
    if !engine_owned(project, state.engine_root.as_deref(), path) {
        return false;
    }
    tracing::error!(
        path = %path.display(),
        "engine assets are read-only: this ships with the engine, and every project \
         on this machine using the same install references it",
    );
    true
}

#[cfg(test)]
mod tests;
