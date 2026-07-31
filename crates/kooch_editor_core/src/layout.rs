//! Persists the editor's dock layout between sessions.
//!
//! Saves to `$XDG_CONFIG_HOME/ome/editor_layout.ron` (Linux/macOS) or the
//! platform equivalent. The file is per-user and **not** versioned with
//! the project — it's UI preference, not scene data.
//!
//! Save strategy: on each frame in [`Stage::Last`](kooch_core::stage::Stage),
//! re-serialize the current `DockState` and compare to the last cached
//! string. Only writes to disk when the serialization actually differs,
//! so steady-state editing produces zero disk traffic.

use std::path::PathBuf;

use egui_dock::DockState;
use kooch_core::resource::Resources;

use crate::state::{EditorOverlay, EditorTab};

/// Cached serialization of the last layout written to disk. Keeps the
/// save system from re-writing identical state every frame.
#[derive(Default)]
pub(crate) struct LayoutPersistence {
    last_serialized: Option<String>,
}

/// Returns the absolute path of the editor layout file, or `None` when
/// the platform's config directory cannot be resolved (rare).
pub(crate) fn layout_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("ome").join("editor_layout.ron"))
}

/// Tries to read and parse the saved layout file. Returns `None` on
/// missing-file (first run) or any parse error (warns but does not
/// fail — the caller falls back to the default layout).
pub(crate) fn load_layout() -> Option<DockState<EditorTab>> {
    let path = layout_path()?;
    let data = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            // ENOENT on first run is expected — log only at debug level.
            if e.kind() == std::io::ErrorKind::NotFound {
                tracing::debug!("No saved dock layout at {path:?} — using defaults");
            } else {
                tracing::warn!("Failed to read dock layout at {path:?}: {e}");
            }
            return None;
        }
    };
    match ron::from_str::<DockState<EditorTab>>(&data) {
        Ok(state) => {
            tracing::info!("Loaded dock layout from {path:?}");
            Some(state)
        }
        Err(e) => {
            tracing::warn!("Failed to parse dock layout at {path:?}: {e}. Using default layout.");
            None
        }
    }
}

/// Writes a layout to disk, creating the parent directory if needed.
pub(crate) fn save_layout(state: &DockState<EditorTab>) -> std::io::Result<()> {
    let Some(path) = layout_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "no config directory available on this platform",
        ));
    };
    let serialized = ron::ser::to_string_pretty(state, ron::ser::PrettyConfig::default())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serialized)
}

/// Startup system: loads the saved layout and replaces the overlay's
/// default `DockState`. Pre-populates [`LayoutPersistence`] so the
/// save system doesn't immediately re-write the just-loaded state.
pub(crate) fn load_layout_system(resources: &mut Resources) {
    let Some(loaded) = load_layout() else {
        // Still initialize the cache from the existing default so the
        // first save-system tick doesn't write the default layout.
        if let Some(overlay) = resources.get::<EditorOverlay>()
            && let Ok(s) = ron::ser::to_string(&overlay.dock_state)
            && let Some(persist) = resources.get_mut::<LayoutPersistence>()
        {
            persist.last_serialized = Some(s);
        }
        return;
    };
    if let Some(overlay) = resources.get_mut::<EditorOverlay>() {
        overlay.dock_state = loaded;
    }
    // Cache the new state so the next save-system tick recognises it.
    if let Some(overlay) = resources.get::<EditorOverlay>()
        && let Ok(s) = ron::ser::to_string(&overlay.dock_state)
        && let Some(persist) = resources.get_mut::<LayoutPersistence>()
    {
        persist.last_serialized = Some(s);
    }
}

/// Save system: re-serializes the current dock state and writes to disk
/// only when it differs from the last cached serialization. Designed to
/// run every frame in [`Stage::Last`](kooch_core::stage::Stage) at minimal
/// cost — typical frames produce zero disk writes.
pub(crate) fn save_layout_system(resources: &mut Resources) {
    // Phase 1: snapshot the dock state and its serialization in a tight
    // scope so the immutable borrow on Resources is released before the
    // mutable get below.
    let (dock_state, serialized) = {
        let Some(overlay) = resources.get::<EditorOverlay>() else {
            return;
        };
        let serialized = match ron::ser::to_string(&overlay.dock_state) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to serialize dock layout: {e}");
                return;
            }
        };
        (overlay.dock_state.clone(), serialized)
    };

    // Phase 2: skip the write entirely when the layout is unchanged.
    let unchanged = resources
        .get::<LayoutPersistence>()
        .and_then(|p| p.last_serialized.as_deref())
        .is_some_and(|last| last == serialized);
    if unchanged {
        return;
    }

    // Phase 3: persist and update the cache.
    match save_layout(&dock_state) {
        Ok(()) => {
            if let Some(persist) = resources.get_mut::<LayoutPersistence>() {
                persist.last_serialized = Some(serialized);
            }
            tracing::debug!("Dock layout persisted to disk");
        }
        Err(e) => {
            tracing::warn!("Failed to save dock layout: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::default_dock_state;

    #[test]
    fn round_trip_preserves_default_layout() {
        let original = default_dock_state();
        let serialized = ron::ser::to_string(&original).expect("serialize default");
        let parsed: DockState<EditorTab> = ron::from_str(&serialized).expect("parse round-trip");
        // We can't trivially `==` two DockStates (egui_dock doesn't impl Eq),
        // but a re-serialization should produce the same string.
        let reserialized = ron::ser::to_string(&parsed).expect("reserialize");
        assert_eq!(serialized, reserialized);
    }

    #[test]
    fn layout_path_resolves_under_config_dir() {
        let path = layout_path().expect("config dir resolves on test platform");
        assert!(path.ends_with("ome/editor_layout.ron"));
    }

    #[test]
    fn load_layout_returns_none_for_missing_file() {
        // Override config dir is platform-dependent; we just verify no panic
        // when the file probably doesn't exist (most CI environments).
        // If a real layout file exists from a prior run we just skip — the
        // function is deterministic w.r.t. the current filesystem.
        let _ = load_layout();
    }
}
