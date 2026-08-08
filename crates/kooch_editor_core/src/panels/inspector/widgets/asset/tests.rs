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
