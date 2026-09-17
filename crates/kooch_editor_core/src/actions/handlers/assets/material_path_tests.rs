//! Test code for `material_path`, in its own file.

use std::time::SystemTime;

use kooch_core::Guid;
use kooch_core::asset_database::{AssetDatabase, AssetEntry};
use kooch_core::resource::Resources;
use kooch_render::material::{MATERIAL_TYPE_NAME, Material, SHADER_TYPE_NAME};

use super::handle_edit_material;

/// Registers a scratch file typed `type_name` and returns its guid and path.
fn asset(resources: &mut Resources, name: &str, type_name: &str) -> (Guid, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!("kooch_material_path_{name}"));
    std::fs::write(&path, "original").unwrap();
    let guid = Guid::new_v4();
    let entry = AssetEntry {
        path: path.clone(),
        mtime: SystemTime::now(),
        type_name: Some(type_name.to_owned()),
    };
    if resources.get::<AssetDatabase>().is_none() {
        resources.insert(AssetDatabase::new());
    }
    resources
        .get_mut::<AssetDatabase>()
        .unwrap()
        .register(guid, entry);
    (guid, path)
}

#[test]
fn a_shader_guid_is_not_overwritten() {
    let mut resources = Resources::new();
    let (shader, shader_path) = asset(&mut resources, "shader", SHADER_TYPE_NAME);
    let (material, material_path) = asset(&mut resources, "material", MATERIAL_TYPE_NAME);

    handle_edit_material(&mut resources, shader, &Material::default(), true);
    handle_edit_material(&mut resources, material, &Material::default(), true);

    assert_eq!(std::fs::read_to_string(&shader_path).unwrap(), "original");
    assert_ne!(std::fs::read_to_string(&material_path).unwrap(), "original");
    let _ = std::fs::remove_file(shader_path);
    let _ = std::fs::remove_file(material_path);
}
