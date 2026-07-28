//! Where gizmo visibility survives a restart.
//!
//! A per-user file rather than project state: which gizmos you keep on is
//! a preference about how you work, not something a scene should carry to
//! whoever opens it next.

use ome_core::resource::Resources;

use super::GizmoVisibility;

/// Where the choices live, next to the dock layout.
///
/// Its own file rather than a field inside `editor_layout.ron`: the layout
/// is rewritten whenever a panel moves, and folding an unrelated setting
/// into it would mean a dragged splitter and a hidden gizmo group sharing
/// one write path — and one corrupt file losing both.
pub(crate) fn visibility_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("ome").join("gizmo_visibility.ron"))
}

/// Reads the saved choices. Missing file or unparseable content both mean
/// "everything visible" — a preference is not worth failing a launch over.
pub(crate) fn load() -> GizmoVisibility {
    let Some(path) = visibility_path() else {
        return GizmoVisibility::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!(
                target: "ome_editor_core::gizmos::visibility",
                path = %path.display(),
                error = %e,
                "unreadable gizmo visibility file; showing everything",
            );
            GizmoVisibility::new()
        }),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    target: "ome_editor_core::gizmos::visibility",
                    path = %path.display(),
                    error = %e,
                    "could not read gizmo visibility",
                );
            }
            GizmoVisibility::new()
        }
    }
}

/// Startup system: pulls the saved choices into `Resources`.
pub(crate) fn load_visibility_system(resources: &mut Resources) {
    let loaded = load();
    resources.insert(VisibilityPersistence {
        last_serialized: ron::ser::to_string(&loaded).ok(),
    });
    resources.insert(loaded);
}

/// Cache of what is already on disk, so a steady-state frame writes
/// nothing. Same shape as `LayoutPersistence`.
#[derive(Default)]
pub(crate) struct VisibilityPersistence {
    last_serialized: Option<String>,
}

/// End-of-frame system: writes the choices when they actually changed.
pub(crate) fn save_visibility_system(resources: &mut Resources) {
    let Some(serialized) = resources
        .get::<GizmoVisibility>()
        .and_then(|v| ron::ser::to_string(v).ok())
    else {
        return;
    };
    let Some(persist) = resources.get_mut::<VisibilityPersistence>() else {
        return;
    };
    if persist.last_serialized.as_deref() == Some(serialized.as_str()) {
        return;
    }
    persist.last_serialized = Some(serialized.clone());

    let Some(path) = visibility_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "ome_editor_core::gizmos::visibility",
            error = %e,
            "could not create the config directory",
        );
        return;
    }
    if let Err(e) = std::fs::write(&path, serialized) {
        tracing::warn!(
            target: "ome_editor_core::gizmos::visibility",
            path = %path.display(),
            error = %e,
            "could not save gizmo visibility",
        );
    }
}
