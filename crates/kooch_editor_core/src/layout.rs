//! Persists the editor's dock layout, and its torn-off panels, between sessions.

use std::path::PathBuf;

use egui_dock::DockState;
use kooch_core::resource::Resources;

use crate::os_windows::Detached;
use crate::state::{EditorOverlay, EditorTab};

/// What the layout file holds: the dock, and the panels torn off into windows (#1196).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct EditorLayout {
    pub dock: DockState<EditorTab>,
    #[serde(default)]
    pub windows: Vec<Detached>,
}

impl EditorLayout {
    fn of(overlay: &EditorOverlay) -> Self {
        Self {
            dock: overlay.dock_state.clone(),
            windows: overlay.windows.detached.clone(),
        }
    }

    /// 🔴 Also reads the bare dock a layout file held before #1196, so an upgrade keeps it.
    pub(crate) fn parse(data: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str::<Self>(data).or_else(|err| {
            ron::from_str::<DockState<EditorTab>>(data)
                .map(|dock| Self {
                    dock,
                    windows: Vec::new(),
                })
                .map_err(|_| err)
        })
    }
}

/// Cached serialization of the last layout written to disk. Keeps the
/// save system from re-writing identical state every frame.
#[derive(Default)]
pub(crate) struct LayoutPersistence {
    last_serialized: Option<String>,
}

/// Returns the absolute path of the editor layout file, or `None` when
/// the platform's config directory cannot be resolved (rare).
pub(crate) fn layout_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("kooch").join("editor_layout.ron"))
}

/// Tries to read and parse the saved layout file. Returns `None` on
/// missing-file (first run) or any parse error (warns but does not
/// fail — the caller falls back to the default layout).
pub(crate) fn load_layout() -> Option<EditorLayout> {
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
    match EditorLayout::parse(&data) {
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
pub(crate) fn save_layout(state: &EditorLayout) -> std::io::Result<()> {
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
            && let Ok(s) = ron::ser::to_string(&EditorLayout::of(overlay))
            && let Some(persist) = resources.get_mut::<LayoutPersistence>()
        {
            persist.last_serialized = Some(s);
        }
        return;
    };
    if let Some(overlay) = resources.get_mut::<EditorOverlay>() {
        overlay.dock_state = loaded.dock;
        overlay.windows.detached = loaded.windows;
        crate::os_windows::settle(&mut overlay.dock_state, &overlay.windows);
    }
    // Cache the new state so the next save-system tick recognises it.
    if let Some(overlay) = resources.get::<EditorOverlay>()
        && let Ok(s) = ron::ser::to_string(&EditorLayout::of(overlay))
        && let Some(persist) = resources.get_mut::<LayoutPersistence>()
    {
        persist.last_serialized = Some(s);
    }
}

/// Save system: re-serializes the current dock state and writes to disk only when it differs from
/// the last cached serialization.
pub(crate) fn save_layout_system(resources: &mut Resources) {
    // Phase 1: snapshot the dock state and its serialization in a tight
    // scope so the immutable borrow on Resources is released before the
    // mutable get below.
    let (layout, serialized) = {
        let Some(overlay) = resources.get::<EditorOverlay>() else {
            return;
        };
        let layout = EditorLayout::of(overlay);
        let serialized = match ron::ser::to_string(&layout) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to serialize dock layout: {e}");
                return;
            }
        };
        (layout, serialized)
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
    match save_layout(&layout) {
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
mod tests;
