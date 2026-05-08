use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::query::Query;
use crate::reflect::ReflectValue;
use crate::scene::{
    sync_scene_to_ecs, ComponentDescription, EntityDescription, SceneDocument,
};
use crate::transform::Transform;

use super::{setup_resources, Health, TestAssetHolder, TestEphemeral};

#[test]
fn despawn_all_preserves_ephemeral_entities() {
    use crate::ephemeral::EphemeralComponents;

    let mut resources = setup_resources();
    let mut ephemeral = EphemeralComponents::new();
    ephemeral.insert(std::any::TypeId::of::<TestEphemeral>());
    resources.insert(ephemeral);

    // Register Health so it can be looked up by name during sync.
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Health>();

    // Spawn one persistent and one ephemeral entity.
    {
        let mut commands = resources.remove::<Commands>().unwrap();
        commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 1, max_hp: 1 });
        commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 99, max_hp: 99 })
            .insert(TestEphemeral);
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    // Loading an empty scene should wipe the persistent entity but
    // keep the ephemeral one alive.
    let empty = SceneDocument {
        name: "empty".into(),
        version: "0.1.0".into(),
        entities: vec![],
    };
    sync_scene_to_ecs(&empty, &mut resources).unwrap();

    let query = Query::<&Health>::new(&resources);
    let healths: Vec<u32> = query.iter().map(|h| h.hp).collect();
    assert_eq!(healths, vec![99], "ephemeral entity must survive scene load");
}

#[test]
fn sync_scene_to_ecs_rebuilds_entities() {
    let mut resources = setup_resources();

    // Register component types so they can be looked up by name.
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu_reflected::<Health>();
        registry.register_cpu_reflected::<Transform>();
    }

    let doc = SceneDocument {
        name: "Test".into(),
        version: "0.1.0".into(),
        entities: vec![EntityDescription {
            name: "Hero".into(),
            parent: None,
            components: vec![ComponentDescription {
                type_name: std::any::type_name::<Health>().to_owned(),
                fields: vec![
                    ("hp".into(), ReflectValue::U32(77)),
                    ("max_hp".into(), ReflectValue::U32(200)),
                ],
            }],
        }],
    };

    sync_scene_to_ecs(&doc, &mut resources).unwrap();

    // Verify the entity exists with correct field values.
    let query = Query::<&Health>::new(&resources);
    let results: Vec<_> = query.iter().collect();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].hp, 77);
    assert_eq!(results[0].max_hp, 200);
}

// -- AssetRef round-trip ---------------------------------------------

/// End-to-end: spawn an entity with both AssetRef fields populated,
/// snapshot the world to a `SceneDocument`, save to RON, load it
/// back, and sync into a fresh world. Both GUIDs must survive the
/// round-trip — otherwise scenes that reference engine assets
/// silently lose their bindings on reload.
#[test]
fn assetref_fields_round_trip_through_scene_save_load() {
    use ome_core::Guid;
    use std::path::PathBuf;

    let mesh_guid = Guid::new_v4();
    let material_guid = Guid::new_v4();

    // 1. Build source world + spawn one entity with both fields.
    let mut src = setup_resources();
    src.get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<TestAssetHolder>();
    {
        let mut commands = src.remove::<Commands>().unwrap();
        commands
            .spawn(&mut src)
            .insert_reflected(TestAssetHolder {
                mesh: Some(mesh_guid),
                material: Some(material_guid),
            });
        commands.apply(&mut src);
        src.insert(commands);
    }

    // 2. Snapshot + RON round-trip via on-disk file.
    let doc = SceneDocument::from_ecs(&src);
    let tmp_dir = std::env::temp_dir();
    let scene_path: PathBuf = tmp_dir.join(format!(
        "ome_assetref_round_trip_{}.ron",
        std::process::id(),
    ));
    doc.save(&scene_path).expect("scene save");
    let reloaded = SceneDocument::load(&scene_path).expect("scene load");
    let _ = std::fs::remove_file(&scene_path);

    // 3. Sync into a fresh world.
    let mut dst = setup_resources();
    dst.get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<TestAssetHolder>();
    sync_scene_to_ecs(&reloaded, &mut dst).expect("sync");

    // 4. Assert the GUIDs round-trip into the new component.
    let query = Query::<&TestAssetHolder>::new(&dst);
    let results: Vec<_> = query.iter().collect();
    assert_eq!(results.len(), 1, "exactly one entity must be reconstructed");
    assert_eq!(
        results[0].mesh,
        Some(mesh_guid),
        "mesh GUID must survive scene round-trip",
    );
    assert_eq!(
        results[0].material,
        Some(material_guid),
        "material GUID must survive scene round-trip",
    );
}

/// `None` AssetRefs must round-trip too — defensive against a
/// serializer that conflates `Some(uuid)` and `None`.
#[test]
fn assetref_none_round_trip() {
    let mut src = setup_resources();
    src.get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<TestAssetHolder>();
    {
        let mut commands = src.remove::<Commands>().unwrap();
        commands
            .spawn(&mut src)
            .insert_reflected(TestAssetHolder::default());
        commands.apply(&mut src);
        src.insert(commands);
    }

    let doc = SceneDocument::from_ecs(&src);
    // Direct in-memory round-trip via RON to avoid the temp file.
    let serialized = ron::ser::to_string(&doc).expect("serialize");
    let reloaded: SceneDocument = ron::from_str(&serialized).expect("deserialize");

    let mut dst = setup_resources();
    dst.get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<TestAssetHolder>();
    sync_scene_to_ecs(&reloaded, &mut dst).expect("sync");

    let query = Query::<&TestAssetHolder>::new(&dst);
    let results: Vec<_> = query.iter().collect();
    assert_eq!(results.len(), 1);
    assert!(results[0].mesh.is_none(), "None mesh must survive round-trip");
    assert!(results[0].material.is_none(), "None material must survive round-trip");
}
