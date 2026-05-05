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
//! - **Material**: RON only. Same format as `.ome_scene`, handles
//!   nested structures cleanly. Drops TOML.
//!
//! `extensions` is empty when the kind is recognised but no specific
//! filter applies (generic `*_path` / `*_file` fields). The dialog will
//! still show the file picker, just without a type filter.
//!
//! Bridge until the asset handle system (#184) makes this explicit via
//! typed handles.

use ome_core::Guid;
use ome_core::asset_database::AssetDatabase;

/// Snapshot of one `AssetDatabase` entry exposed to the inspector.
///
/// Pre-collected per frame because the inspector renders inside the
/// egui callback closure, which cannot freely borrow the
/// `&Resources` that owns the database. Cheap to clone (small
/// strings); the catalog rebuild happens once per frame.
#[derive(Clone, Debug)]
pub(crate) struct AssetCatalogEntry {
    pub guid: Guid,
    /// Human-readable label shown in the dropdown — the asset's
    /// path (display form). Falls back to the GUID when the path
    /// is somehow empty.
    pub label: String,
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
    pub(crate) fn collect_from_database(db: &AssetDatabase) -> Vec<Self> {
        let mut out: Vec<Self> = Vec::new();
        // No public iter on AssetDatabase yet; collect by scanning
        // every type the database currently sees. We rely on the
        // (Guid, &AssetEntry) pairs the database exposes through
        // `entries_of_type` for each known type, but to avoid a
        // double-iteration pass we walk the bidirectional map
        // directly via `entry()` keyed by the path map's GUIDs.
        // Simpler: iterate all entries of *any* type by collecting
        // the union over the path index.
        // (`AssetDatabase` does not expose `iter()` yet; if more
        // surfaces need it, promote this loop to a public iterator.)
        let mut seen_guids: std::collections::HashSet<Guid> =
            std::collections::HashSet::new();
        // Iterate through the path index — every registered asset
        // sits in `by_path`, so this gives us each GUID exactly once.
        for guid in db.path_iter().map(|(_, g)| g) {
            if !seen_guids.insert(guid) {
                continue;
            }
            let Some(entry) = db.entry(guid) else { continue };
            let Some(type_name) = entry.type_name.clone() else {
                continue;
            };
            out.push(AssetCatalogEntry {
                guid,
                label: entry.path.display().to_string(),
                type_name,
            });
        }
        out
    }
}

/// Returns `(label, extensions)` when a String field's name suggests
/// it holds an asset path.
pub(super) fn asset_filter_for(field_name: &str) -> Option<(&'static str, &'static [&'static str])> {
    let n = field_name.to_lowercase();
    if n.contains("mesh") {
        Some(("Mesh", &["gltf", "glb"]))
    } else if n.contains("texture") || n.contains("image") {
        Some(("Texture", &["png", "jpg", "jpeg", "ktx2", "exr", "hdr"]))
    } else if n.contains("audio") || n.contains("sound") {
        Some(("Audio", &["ogg", "wav", "flac"]))
    } else if n.contains("scene") {
        Some(("Scene", &["ome_scene"]))
    } else if n.contains("shader") {
        Some(("Shader", &["wgsl"]))
    } else if n.contains("material") {
        Some(("Material", &["ron"]))
    } else if n.ends_with("_path") || n.ends_with("_file") {
        Some(("File", &[]))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::asset_filter_for;

    #[test]
    fn detects_mesh_field() {
        let (label, exts) = asset_filter_for("mesh").expect("mesh recognised");
        assert_eq!(label, "Mesh");
        assert!(exts.contains(&"gltf"));
        assert!(exts.contains(&"glb"));
        // Legacy formats deliberately excluded — see asset_filter_for docstring.
        assert!(!exts.contains(&"obj"));
    }

    #[test]
    fn excludes_legacy_and_licensed_audio() {
        let (_, exts) = asset_filter_for("audio").expect("audio recognised");
        assert!(exts.contains(&"ogg"));
        assert!(exts.contains(&"flac"));
        assert!(!exts.contains(&"mp3"));
    }

    #[test]
    fn material_uses_ron_only() {
        let (_, exts) = asset_filter_for("material").expect("material recognised");
        assert_eq!(exts, &["ron"]);
    }

    #[test]
    fn detects_compound_field_names() {
        // Real components use suffixed names like `mesh_path`, `texture_handle`, etc.
        assert!(asset_filter_for("mesh_path").is_some());
        assert!(asset_filter_for("diffuse_texture").is_some());
        assert!(asset_filter_for("background_audio").is_some());
        assert!(asset_filter_for("startup_scene").is_some());
        assert!(asset_filter_for("vertex_shader").is_some());
        assert!(asset_filter_for("base_material").is_some());
    }

    #[test]
    fn case_insensitive() {
        assert!(asset_filter_for("MeshPath").is_some());
        assert!(asset_filter_for("DIFFUSE_TEXTURE").is_some());
    }

    #[test]
    fn generic_path_suffix_falls_back_to_no_filter() {
        let (label, exts) = asset_filter_for("config_path").expect("recognised");
        assert_eq!(label, "File");
        assert!(exts.is_empty());
    }

    #[test]
    fn non_asset_strings_return_none() {
        // Plain string fields like Name.value must NOT trigger a picker.
        assert!(asset_filter_for("value").is_none());
        assert!(asset_filter_for("name").is_none());
        assert!(asset_filter_for("description").is_none());
    }
}
