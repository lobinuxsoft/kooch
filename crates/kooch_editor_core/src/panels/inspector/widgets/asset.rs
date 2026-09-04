//! Asset-path detection: name-based heuristic that maps `String`
//! reflected fields like `mesh_path` / `texture_handle` to a file
//! picker dialog filter — plus the typed [`AssetCatalogEntry`] the
//! `ReflectValue::AssetRef` widget consumes when rendering its
//! `AssetDatabase`-backed dropdown.
//!
//! Format choices favour open standards with no licensing friction and
//! formats the engine actually plans to support:
//! - **Mesh**: glTF 2.0 only. Khronos open spec, supports animation,
//!   skeletons, materials, scenes — drops the legacy `.obj`.
//! - **Texture**: LDR (PNG, JPEG), GPU compressed (KTX2), HDR (EXR,
//!   Radiance HDR). Drops `.tga` — legacy with no real advantage.
//! - **Audio**: Xiph (Vorbis, FLAC) plus PCM `.wav`. Drops `.mp3`
//!   because `kira` gates it behind a feature flag and the historical
//!   patent baggage adds zero value when Vorbis covers the same niche.
//! - **Material**: RON only. Same format as `.scene`, handles
//!   nested structures cleanly. Drops TOML.
//!
//! `extensions` is empty when the kind is recognised but no specific
//! filter applies (generic `*_path` / `*_file` fields). The dialog will
//! still show the file picker, just without a type filter.
//!
//! Bridge until the asset handle system (#184) makes this explicit via
//! typed handles.

use std::path::{Path, PathBuf};

use kooch_core::Guid;
use kooch_core::asset_database::AssetDatabase;

/// Origin of an asset relative to the project / engine boundary.
/// Drives the inspector picker's `[engine]` / `[project]` tag so
/// users can spot at a glance whether they are referencing a
/// shipped engine asset (Suzanne, default sky textures) or a
/// project-local one. Falls back to `Other` when neither root
/// matches — useful for assets the editor surfaces from a third
/// location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssetSource {
    Engine,
    Project,
    Other,
}

impl AssetSource {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::Project => "project",
            Self::Other => "other",
        }
    }
}

/// Snapshot of one `AssetDatabase` entry exposed to the inspector.
///
/// Pre-collected per frame because the inspector renders inside the
/// egui callback closure, which cannot freely borrow the
/// `&Resources` that owns the database. Cheap to clone (small
/// strings); the catalog rebuild happens once per frame.
#[derive(Clone, Debug)]
pub(crate) struct AssetCatalogEntry {
    pub guid: Guid,
    /// Full project-relative path. Surfaced on hover and used by the
    /// picker's search filter for substring matching.
    pub path: PathBuf,
    /// Short display name shown by default in the picker — the
    /// file's basename. Falls back to the path's full display form
    /// when the basename is unavailable (very rare).
    pub display_name: String,
    /// Where this asset lives. Tag rendered next to the picker's
    /// label as `[engine]` / `[project]` / `[other]`.
    pub source: AssetSource,
    /// The asset's recorded type, used by the picker to filter the
    /// catalog by the field's `asset_type`. Untyped entries (no
    /// `load::<T>` ever ran) are skipped during collection.
    pub type_name: String,
}

impl AssetCatalogEntry {
    /// Collects every typed entry from `db`. Untyped entries
    /// (sidecars whose `asset_type` is `None`) are skipped — the
    /// inspector picker has no way to filter them, so listing them
    /// would leak unfiltered noise into every typed dropdown.
    ///
    /// `engine_root` and `project_root` are the asset directories
    /// the host knows about (typically `<engine>/assets` and
    /// `<project>/assets`). Either may be `None`; entries whose
    /// path matches neither are tagged `AssetSource::Other`.
    pub(crate) fn collect_from_database(
        db: &AssetDatabase,
        engine_root: Option<&Path>,
        project_root: Option<&Path>,
    ) -> Vec<Self> {
        let mut out: Vec<Self> = Vec::new();
        let mut seen_guids: std::collections::HashSet<Guid> = std::collections::HashSet::new();
        for guid in db.path_iter().map(|(_, g)| g) {
            if !seen_guids.insert(guid) {
                continue;
            }
            let Some(entry) = db.entry(guid) else {
                continue;
            };
            let Some(type_name) = entry.type_name.clone() else {
                continue;
            };
            let display_name = entry
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_owned())
                .unwrap_or_else(|| entry.path.display().to_string());
            let source = classify_source(&entry.path, engine_root, project_root);
            out.push(AssetCatalogEntry {
                guid,
                path: entry.path.clone(),
                display_name,
                source,
                type_name,
            });
        }
        // Stable presentation: engine first, then project, then
        // others; alphabetical within each group. The picker user
        // expects the same asset to land in the same row across
        // frames.
        out.sort_by(|a, b| {
            (a.source as u8, a.display_name.as_str())
                .cmp(&(b.source as u8, b.display_name.as_str()))
        });
        out
    }
}

fn classify_source(
    asset_path: &Path,
    engine_root: Option<&Path>,
    project_root: Option<&Path>,
) -> AssetSource {
    if let Some(root) = engine_root
        && asset_path.starts_with(root)
    {
        return AssetSource::Engine;
    }
    if let Some(root) = project_root
        && asset_path.starts_with(root)
    {
        return AssetSource::Project;
    }
    AssetSource::Other
}

/// Returns `(label, extensions)` when a String field's name suggests
/// it holds an asset path.
pub(super) fn asset_filter_for(
    field_name: &str,
) -> Option<(&'static str, &'static [&'static str])> {
    let n = field_name.to_lowercase();
    if n.contains("mesh") {
        Some(("Mesh", &["gltf", "glb"]))
    } else if n.contains("texture") || n.contains("image") {
        Some(("Texture", &["png", "jpg", "jpeg", "ktx2", "exr", "hdr"]))
    } else if n.contains("audio") || n.contains("sound") {
        Some(("Audio", &["ogg", "wav", "flac"]))
    } else if n.contains("scene") {
        Some(("Scene", &["scene"]))
    } else if n.contains("shader") {
        Some(("Shader", &["wgsl"]))
    } else if n.contains("material") {
        Some(("Material", &[kooch_render::material::MATERIAL_EXTENSION]))
    } else if n.ends_with("_path") || n.ends_with("_file") {
        Some(("File", &[]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
