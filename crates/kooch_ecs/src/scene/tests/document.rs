use std::any::TypeId;

use glam::Vec3;

use crate::commands::Commands;
use crate::component::ComponentRegistry;
use crate::reflect::ReflectValue;
use crate::scene::{ComponentDescription, EntityDescription, SceneDocument};
use crate::transform::Transform;

use super::{Health, TestEphemeral, add_to_archetype, setup_resources};

#[test]
fn round_trip_save_load() {
    let doc = SceneDocument {
        id: kooch_core::Guid::new_v4(),
        name: "Test Scene".into(),
        version: "0.1.0".into(),
        entities: vec![
            EntityDescription {
                name: "Player".into(),
                parent_index: None,
                parent: None,
                components: vec![ComponentDescription {
                    type_name: "kooch_ecs::transform::Transform".into(),
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

    let dir = std::env::temp_dir().join("kooch_scene_test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("round_trip.scene");

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

/// Membership is reflected, and must still never reach the file.
///
/// Reflecting `SceneMember` is what lets a world rebuild carry it — see
/// `WorldSnapshot`. It also makes it eligible for every generic "write
/// the reflected components" pass, and `from_ecs` is one. Writing it
/// would state membership twice, and with several copies of a scene open
/// the value on the entity is the *instance* guid — so the file would
/// come back naming a scene that only existed in one session.
#[test]
fn a_saved_scene_never_states_its_own_membership() {
    use crate::scene_member::SceneMember;

    let mut resources = setup_resources();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<SceneMember>();
    registry.register_cpu_reflected::<crate::name::Name>();

    let mut commands = resources.remove::<Commands>().unwrap();
    let entity = commands.spawn(&mut resources).id();
    commands.apply(&mut resources);
    resources.insert(commands);

    let scene = kooch_core::Guid::new_v4();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<crate::name::Name>()
        .unwrap()
        .insert(entity, crate::name::Name::new("Rig"));
    registry
        .get_cpu_mut::<Transform>()
        .unwrap()
        .insert(entity, Transform::default());
    registry
        .get_cpu_mut::<SceneMember>()
        .unwrap()
        .insert(entity, SceneMember::new(scene));
    add_to_archetype(&mut resources, entity, TypeId::of::<crate::name::Name>());
    add_to_archetype(&mut resources, entity, TypeId::of::<Transform>());
    add_to_archetype(&mut resources, entity, TypeId::of::<SceneMember>());

    let doc = SceneDocument::from_ecs(&mut resources);

    let written: Vec<&str> = doc
        .entities
        .iter()
        .flat_map(|e| e.components.iter())
        .map(|c| c.type_name.as_str())
        .collect();
    assert!(
        !written.iter().any(|name| name.contains("SceneMember")),
        "membership reached the file: {written:?}"
    );
    // The entity itself still has to be there, or the assertion above
    // would pass on an empty document.
    assert_eq!(doc.entities.len(), 1);
}
