//! Asset-path detection: name-based heuristic that maps `String`
//! reflected fields like `mesh_path` / `texture_handle` to a file
//! picker dialog filter.
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
