use glam::Vec3;

use crate::commands::Commands;
use crate::reflect::ReflectValue;
use crate::scene::{ComponentDescription, EntityDescription, SceneDocument};
use crate::transform::Transform;

use super::{Health, TestEphemeral, setup_resources};

#[test]
fn round_trip_save_load() {
    let doc = SceneDocument {
        id: ome_core::Guid::new_v4(),
        name: "Test Scene".into(),
        version: "0.1.0".into(),
        entities: vec![
            EntityDescription {
                name: "Player".into(),
                parent_index: None,
                parent: None,
                components: vec![ComponentDescription {
                    type_name: "ome_ecs::transform::Transform".into(),
                    fields: vec![
                        (
                            "position".into(),
                            ReflectValue::Vec3(Vec3::new(1.0, 2.0, 3.0)),
                        ),
                        ("rotation".into(), ReflectValue::Quat(glam::Quat::IDENTITY)),
                        ("scale".into(), ReflectValue::Vec3(Vec3::ONE)),
                    ],
                }],
            },
            EntityDescription {
                name: "Enemy".into(),
                parent_index: None,
                parent: None,
                components: vec![ComponentDescription {
                    type_name: "test::Health".into(),
                    fields: vec![
                        ("hp".into(), ReflectValue::U32(42)),
                        ("max_hp".into(), ReflectValue::U32(100)),
                    ],
                }],
            },
        ],
    };

    let dir = std::env::temp_dir().join("ome_scene_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("round_trip.ome_scene");

    doc.save(&path).unwrap();
    let loaded = SceneDocument::load(&path).unwrap();
    assert_eq!(doc, loaded);

    // Cleanup.
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn from_ecs_captures_entities() {
    let mut resources = setup_resources();

    // Spawn two entities with reflected components.
    {
        let mut commands = resources.remove::<Commands>().unwrap();
        commands
            .spawn(&mut resources)
            .insert_reflected(Health {
                hp: 42,
                max_hp: 100,
            })
            .insert_reflected(Transform::from_position(Vec3::new(1.0, 2.0, 3.0)));
        commands
            .spawn(&mut resources)
            .insert_reflected(Health { hp: 10, max_hp: 50 });
        commands.apply(&mut resources);
        resources.insert(commands);
    }

    let doc = SceneDocument::from_ecs(&mut resources);

    assert_eq!(doc.entities.len(), 2);

    // First entity should have 2 components (Health + Transform).
    let e0 = &doc.entities[0];
    assert_eq!(e0.components.len(), 2);

    // Second entity should have 1 component (Health).
    let e1 = &doc.entities[1];
    assert_eq!(e1.components.len(), 1);

    // Verify Health field values on the first entity.
    let health_comp = e0
        .components
        .iter()
        .find(|c| c.type_name.contains("Health"))
        .expect("Health component not found");
    assert!(
        health_comp
            .fields
            .contains(&("hp".into(), ReflectValue::U32(42)))
    );
    assert!(
        health_comp
            .fields
            .contains(&("max_hp".into(), ReflectValue::U32(100)))
    );
}

#[test]
fn from_ecs_skips_ephemeral_entities() {
    use crate::ephemeral::EphemeralComponents;

    let mut resources = setup_resources();
    let mut ephemeral = EphemeralComponents::new();
    ephemeral.insert(std::any::TypeId::of::<TestEphemeral>());
    resources.insert(ephemeral);

    // Spawn one persistent entity and one ephemeral entity.
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

    let doc = SceneDocument::from_ecs(&mut resources);

    // Only the non-ephemeral entity should be serialized.
    assert_eq!(doc.entities.len(), 1);
    let health = &doc.entities[0].components[0];
    assert!(health.fields.contains(&("hp".into(), ReflectValue::U32(1))));
}
