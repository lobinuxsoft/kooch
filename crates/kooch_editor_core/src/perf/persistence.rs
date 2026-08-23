//! Where the viewport overlay choices survive a restart.
//!
//! A per-user file rather than project state, and its OWN file rather
//! than a field inside `editor_layout.ron`, for the same reason gizmo
//! visibility has one: the layout is rewritten whenever a panel moves,
//! and folding an unrelated setting into it would mean a dragged
//! splitter and an overlay toggle sharing one write path — and one
//! corrupt file losing both.
//!
//! The transient fields of [`HudVisibility`] — `panel_visible`, which
//! flickers every frame, and `system_section`, which the panel
//! re-asserts — are `#[serde(skip)]`: what persists is what the user
//! chose in the View menu, not what the frame happened to be doing.

use kooch_core::resource::Resources;

use super::HudVisibility;

/// Where the choices live, next to the dock layout.
pub(crate) fn overlays_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|dir| dir.join("kooch").join("hud_overlays.ron"))
}

/// Reads the saved choices. Missing file or unparseable content both
/// mean the defaults — only the frame-time card on — because a
/// preference is not worth failing a launch over.
pub(crate) fn load() -> HudVisibility {
    let Some(path) = overlays_path() else {
        return HudVisibility::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => ron::from_str(&text).unwrap_or_else(|e| {
            tracing::warn!(
                target: "kooch_editor_core::perf::persistence",
                path = %path.display(),
                error = %e,
                "unreadable overlay file; using the defaults",
            );
            HudVisibility::default()
        }),
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    target: "kooch_editor_core::perf::persistence",
                    path = %path.display(),
                    error = %e,
                    "could not read the overlay choices",
                );
            }
            HudVisibility::default()
        }
    }
}

/// Startup system: pulls the saved choices into `Resources`.
pub(crate) fn load_overlays_system(resources: &mut Resources) {
    let loaded = load();
    resources.insert(OverlayPersistence {
        last_serialized: ron::ser::to_string(&loaded).ok(),
    });
    resources.insert(loaded);
}

/// Cache of what is already on disk, so a steady-state frame writes
/// nothing. Same shape as `LayoutPersistence`.
#[derive(Default)]
pub(crate) struct OverlayPersistence {
    last_serialized: Option<String>,
}

/// End-of-frame system: writes the choices when they actually changed.
/// The skipped fields never reach the string, so a frame that only
/// flipped `panel_visible` compares equal and costs nothing.
pub(crate) fn save_overlays_system(resources: &mut Resources) {
    let Some(serialized) = resources
        .get::<HudVisibility>()
        .and_then(|v| ron::ser::to_string(v).ok())
    else {
        return;
    };
    let Some(persist) = resources.get_mut::<OverlayPersistence>() else {
        return;
    };
    if persist.last_serialized.as_deref() == Some(serialized.as_str()) {
        return;
    }
    persist.last_serialized = Some(serialized.clone());

    let Some(path) = overlays_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "kooch_editor_core::perf::persistence",
            error = %e,
            "could not create the config directory",
        );
        return;
    }
    if let Err(e) = std::fs::write(&path, serialized) {
        tracing::warn!(
            target: "kooch_editor_core::perf::persistence",
            path = %path.display(),
            error = %e,
            "could not save the overlay choices",
        );
    }
}

#[cfg(test)]
mod tests;
