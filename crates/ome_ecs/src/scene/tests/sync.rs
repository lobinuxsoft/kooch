use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::query::Query;
use crate::reflect::ReflectValue;
use crate::scene::{ComponentDescription, EntityDescription, SceneDocument, sync_scene_to_ecs};
use crate::transform::Transform;

use super::{Health, TestAssetHolder, TestEphemeral, setup_resources};

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
    assert_eq!(
        healths,
        vec![99],
        "ephemeral entity must survive scene load"
    );
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
            parent_index: None,
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
        commands.spawn(&mut src).insert_reflected(TestAssetHolder {
            mesh: Some(mesh_guid),
            material: Some(material_guid),
        });
        commands.apply(&mut src);
        src.insert(commands);
    }

    // 2. Snapshot + RON round-trip via on-disk file.
    let doc = SceneDocument::from_ecs(&mut src);
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

    let doc = SceneDocument::from_ecs(&mut src);
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
    assert!(
        results[0].mesh.is_none(),
        "None mesh must survive round-trip"
    );
    assert!(
        results[0].material.is_none(),
        "None material must survive round-trip"
    );
}

// -- Unknown components ------------------------------------------------
//
// Which component types resolve depends on which binary opened the
// scene: a project's own editor build knows its gameplay components,
// the standalone hub never will. An unresolved name must therefore cost
// nothing — not the load, not the neighbouring components, and above
// all not the data itself on the next save.

/// A scene naming a type this binary has no Rust type for must still
/// load, and the components around it must survive.
#[test]
fn unknown_component_does_not_fail_the_load() {
    let mut resources = setup_resources();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Health>();

    let doc = SceneDocument {
        name: "with unknown".into(),
        version: "0.1.0".into(),
        entities: vec![EntityDescription {
            name: "Player".into(),
            parent_index: None,
            parent: None,
            components: vec![
                ComponentDescription {
                    type_name: "game::movement::MoveComponent".into(),
                    fields: vec![("speed".into(), ReflectValue::F32(4.5))],
                },
                ComponentDescription {
                    type_name: std::any::type_name::<Health>().into(),
                    fields: vec![
                        ("hp".into(), ReflectValue::U32(30)),
                        ("max_hp".into(), ReflectValue::U32(50)),
                    ],
                },
            ],
        }],
    };

    sync_scene_to_ecs(&doc, &mut resources).expect("unknown component must not fail the load");

    let query = Query::<&Health>::new(&resources);
    let healths: Vec<(u32, u32)> = query.iter().map(|h| (h.hp, h.max_hp)).collect();
    assert_eq!(
        healths,
        vec![(30, 50)],
        "the known component must load despite an unknown sibling"
    );
}

/// The dangerous case: load in a binary that cannot resolve a component,
/// then save. The unresolved component must come back out byte-for-byte,
/// or opening a project from the hub silently strips its gameplay data.
#[test]
fn unknown_component_survives_a_save_round_trip() {
    use crate::dynamic_components::DynamicComponents;

    let mut resources = setup_resources();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Health>();

    let unknown = ComponentDescription {
        type_name: "game::movement::MoveComponent".into(),
        fields: vec![
            ("speed".into(), ReflectValue::F32(4.5)),
            ("enabled".into(), ReflectValue::Bool(true)),
        ],
    };
    let doc = SceneDocument {
        name: "with unknown".into(),
        version: "0.1.0".into(),
        entities: vec![EntityDescription {
            name: "Player".into(),
            parent_index: None,
            parent: None,
            components: vec![
                unknown.clone(),
                ComponentDescription {
                    type_name: std::any::type_name::<Health>().into(),
                    fields: vec![
                        ("hp".into(), ReflectValue::U32(30)),
                        ("max_hp".into(), ReflectValue::U32(50)),
                    ],
                },
            ],
        }],
    };

    sync_scene_to_ecs(&doc, &mut resources).unwrap();
    assert_eq!(
        resources.get::<DynamicComponents>().unwrap().len(),
        1,
        "the unresolved component must be parked, not dropped"
    );

    let saved = SceneDocument::from_ecs(&mut resources);
    let entity = saved
        .entities
        .iter()
        .find(|e| {
            e.components
                .iter()
                .any(|c| c.type_name == unknown.type_name)
        })
        .expect("saved scene must still carry the unresolved component");
    let round_tripped = entity
        .components
        .iter()
        .find(|c| c.type_name == unknown.type_name)
        .unwrap();
    assert_eq!(
        round_tripped, &unknown,
        "the parked component must round-trip unchanged"
    );
}

/// Loading a second scene must not leak the first scene's parked
/// components onto the new entities.
#[test]
fn parked_components_are_cleared_between_loads() {
    use crate::dynamic_components::DynamicComponents;

    let mut resources = setup_resources();

    let doc = SceneDocument {
        name: "first".into(),
        version: "0.1.0".into(),
        entities: vec![EntityDescription {
            name: "Player".into(),
            parent_index: None,
            parent: None,
            components: vec![ComponentDescription {
                type_name: "game::movement::MoveComponent".into(),
                fields: vec![("speed".into(), ReflectValue::F32(4.5))],
            }],
        }],
    };
    sync_scene_to_ecs(&doc, &mut resources).unwrap();
    assert_eq!(resources.get::<DynamicComponents>().unwrap().len(), 1);

    let empty = SceneDocument {
        name: "second".into(),
        version: "0.1.0".into(),
        entities: vec![],
    };
    sync_scene_to_ecs(&empty, &mut resources).unwrap();
    assert!(
        resources.get::<DynamicComponents>().unwrap().is_empty(),
        "parked components must not outlive the entities they belong to"
    );
}

/// Entity names are not unique, and the scene format used to resolve
/// parents by name.
///
/// A scene with several meshes called "Mesh" is ordinary — TEST3 ships five.
/// Resolving a parent through a `HashMap<String, Entity>` collapses them onto
/// one key, so every child ends up under whichever one was inserted last:
/// the hierarchy is silently rebuilt wrong on load, attached to the wrong
/// entity. Indices are unique; names never were.
#[test]
fn duplicate_names_do_not_confuse_the_hierarchy() {
    use crate::hierarchy::Parent;

    let mut resources = setup_resources();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Health>();

    // Two possible parents with the *same* name, and a child under the
    // first. Resolving by name would attach it to the second.
    let scene = SceneDocument {
        name: "Ambiguous".into(),
        version: "0.1.0".into(),
        entities: vec![
            EntityDescription {
                name: "Mesh".into(),
                parent_index: None,
                parent: None,
                components: vec![ComponentDescription {
                    type_name: std::any::type_name::<Health>().into(),
                    fields: vec![("hp".into(), ReflectValue::U32(1))],
                }],
            },
            EntityDescription {
                name: "Mesh".into(),
                parent_index: None,
                parent: None,
                components: vec![ComponentDescription {
                    type_name: std::any::type_name::<Health>().into(),
                    fields: vec![("hp".into(), ReflectValue::U32(2))],
                }],
            },
            EntityDescription {
                name: "Child".into(),
                parent_index: Some(0),
                parent: None,
                components: vec![],
            },
        ],
    };

    sync_scene_to_ecs(&scene, &mut resources).expect("sync");

    // Which entity landed at which document index.
    let spawned: Vec<crate::entity::Entity> = {
        let registry = resources.get::<ComponentRegistry>().unwrap();
        let health = registry.get_cpu::<Health>().unwrap();
        let mut by_hp: Vec<(u32, crate::entity::Entity)> =
            health.iter().map(|(e, h)| (h.hp, *e)).collect();
        by_hp.sort_by_key(|(hp, _)| *hp);
        by_hp.into_iter().map(|(_, e)| e).collect()
    };
    assert_eq!(spawned.len(), 2, "both meshes should exist");
    let first_mesh = spawned[0];
    let second_mesh = spawned[1];

    let registry = resources.get::<ComponentRegistry>().unwrap();
    let parents = registry.get_cpu::<Parent>().expect("no Parent storage");
    let (_, parent_comp) = parents.iter().next().expect("the child was never parented");

    assert_eq!(
        parent_comp.entity, first_mesh,
        "the child was attached to the wrong entity — index 0 was asked for"
    );
    assert_ne!(parent_comp.entity, second_mesh);
}

/// A save/load round-trip preserves which entity is the parent, even when
/// the names collide. This is the shape the bug was actually reported in:
/// save a scene, load it, and the hierarchy comes back attached elsewhere.
#[test]
fn a_round_trip_keeps_the_hierarchy_when_names_collide() {
    use crate::hierarchy::Parent;

    let mut resources = setup_resources();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Health>();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Parent>();

    // Build the world directly: two same-named entities, child under the
    // first one, distinguished by hp.
    let (first, second, child) = {
        let mut commands = resources.remove::<Commands>().unwrap();
        let a = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 1, max_hp: 1 })
            .id();
        let b = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 2, max_hp: 2 })
            .id();
        let c = commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 3, max_hp: 3 })
            .id();
        commands.apply(&mut resources);
        resources.insert(commands);
        (a, b, c)
    };
    for (entity, name) in [(first, "Mesh"), (second, "Mesh"), (child, "Child")] {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.insert_default_reflected(&std::any::TypeId::of::<crate::name::Name>(), entity);
        if let Some(storage) = registry.get_cpu_mut::<crate::name::Name>()
            && let Some(n) = storage.get_mut(entity)
        {
            n.value = name.to_owned();
        }
        super::add_to_archetype(
            &mut resources,
            entity,
            std::any::TypeId::of::<crate::name::Name>(),
        );
    }
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        if let Some(storage) = registry.get_cpu_mut::<Parent>() {
            storage.insert(child, Parent { entity: first });
        }
    }
    super::add_to_archetype(&mut resources, child, std::any::TypeId::of::<Parent>());

    // Save, then load into the same world.
    let document = SceneDocument::from_ecs(&mut resources);
    // Found by the link itself, not by name — the whole point is that names
    // do not identify anything. Since #607 the link is an ordinary
    // component field holding an entity reference, not an out-of-band
    // index into the document.
    let child_desc = document
        .entities
        .iter()
        .find(|e| {
            e.components.iter().any(|c| {
                c.type_name.ends_with("Parent")
                    && c.fields.iter().any(|(_, value)| {
                        matches!(value, ReflectValue::EntityRef(Some(r)) if r.is_unresolved())
                    })
            })
        })
        .expect("the parent link was not saved as an entity reference");
    assert!(
        child_desc.parent.is_none() && child_desc.parent_index.is_none(),
        "a legacy parent link is still being written",
    );

    sync_scene_to_ecs(&document, &mut resources).expect("sync");

    // The reloaded child points at the entity whose hp is 1 — the first
    // "Mesh" — not merely at *some* entity called "Mesh".
    let registry = resources.get::<ComponentRegistry>().unwrap();
    let health = registry.get_cpu::<Health>().unwrap();
    let parents = registry.get_cpu::<Parent>().unwrap();
    let (_, parent_comp) = parents.iter().next().expect("hierarchy lost");
    assert_eq!(
        health.get(parent_comp.entity).map(|h| h.hp),
        Some(1),
        "the round-trip reattached the child to the wrong entity"
    );
}
