//! `RemoteMirror` reconstructs a remote snapshot into a local ECS,
//! keyed by entity id, parking unknown project components.

use ome_core::resource::Resources;
use ome_ecs::allocator::EntityAllocator;
use ome_ecs::archetype_registry::ArchetypeRegistry;
use ome_ecs::commands::Commands;
use ome_ecs::component::ComponentRegistry;
use ome_ecs::dynamic_components::DynamicComponents;
use ome_ecs::hierarchy::Parent;
use ome_ecs::query::AccessTracker;
use ome_ecs::reflect::ReflectValue;
use ome_ecs::transform::Transform;

use ome_editor_core::remote_mirror::RemoteMirror;
use ome_remote::protocol::{ComponentSnapshot, EntityId, EntitySnapshot};

fn ecs() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    r.get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Transform>();
    r
}

fn eid(index: u32) -> EntityId {
    EntityId {
        index,
        generation: 0,
    }
}

/// A parent `Rig` and its child `Mesh`; the child carries a known
/// `Transform` and an unknown project component.
fn snapshot() -> Vec<EntitySnapshot> {
    vec![
        EntitySnapshot {
            id: eid(0),
            name: Some("Rig".into()),
            parent: None,
            components: vec![],
        },
        EntitySnapshot {
            id: eid(1),
            name: Some("Mesh".into()),
            parent: Some(eid(0)),
            components: vec![
                ComponentSnapshot {
                    type_name: std::any::type_name::<Transform>().into(),
                    fields: vec![("position".into(), ReflectValue::Vec3(glam::Vec3::X))],
                },
                ComponentSnapshot {
                    type_name: "game::spin::Spin".into(),
                    fields: vec![("rpm".into(), ReflectValue::F32(33.0))],
                },
            ],
        },
    ]
}

#[test]
fn apply_rebuilds_entities_hierarchy_and_parks_unknowns() {
    let mut resources = ecs();
    let mut mirror = RemoteMirror::new();

    mirror.apply(&snapshot(), &mut resources);

    // Both entities exist locally.
    let rig = mirror.local_of(eid(0)).expect("rig mirrored");
    let mesh = mirror.local_of(eid(1)).expect("mesh mirrored");
    assert_ne!(rig, mesh);

    // The known Transform loaded with its field value.
    let position = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(mesh))
        .map(|t| t.position);
    assert_eq!(position, Some(glam::Vec3::X));

    // Hierarchy is reconstructed by id, not name.
    let parent = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Parent>())
        .and_then(|s| s.get(mesh))
        .map(|p| p.entity);
    assert_eq!(parent, Some(rig));

    // The unknown project component is parked, not dropped.
    let parked: Vec<_> = resources
        .get::<DynamicComponents>()
        .unwrap()
        .iter_entity(mesh)
        .map(|(name, _)| name.to_owned())
        .collect();
    assert_eq!(parked, vec!["game::spin::Spin".to_owned()]);
}

#[test]
fn reapply_replaces_the_previous_mirror() {
    let mut resources = ecs();
    let mut mirror = RemoteMirror::new();

    mirror.apply(&snapshot(), &mut resources);

    // A second snapshot with a single, different entity.
    let next = vec![EntitySnapshot {
        id: eid(7),
        name: Some("Solo".into()),
        parent: None,
        components: vec![ComponentSnapshot {
            type_name: std::any::type_name::<Transform>().into(),
            fields: vec![],
        }],
    }];
    mirror.apply(&next, &mut resources);

    // The old ids are gone; the new one is present.
    assert!(mirror.local_of(eid(0)).is_none());
    assert!(mirror.local_of(eid(1)).is_none());
    let solo = mirror.local_of(eid(7)).expect("solo mirrored");

    // Exactly one entity carries a Transform now (the old two are gone).
    let count = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .map(|s| s.iter().count())
        .unwrap_or(0);
    assert_eq!(count, 1);
    assert!(
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Transform>())
            .and_then(|s| s.get(solo))
            .is_some()
    );
}

/// Re-applying keeps the same local entity for the same remote id, so
/// the editor's selection survives a refresh.
#[test]
fn reapply_keeps_local_entities_stable() {
    let mut resources = ecs();
    let mut mirror = RemoteMirror::new();

    mirror.apply(&snapshot(), &mut resources);
    let before = mirror.local_of(eid(1)).expect("mesh mirrored");

    // The project moved the mesh; everything else is unchanged.
    let mut next = snapshot();
    next[1].components[0].fields = vec![("position".into(), ReflectValue::Vec3(glam::Vec3::Y))];
    mirror.apply(&next, &mut resources);

    assert_eq!(
        mirror.local_of(eid(1)),
        Some(before),
        "entity handle churned"
    );
    let position = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(before))
        .map(|t| t.position);
    assert_eq!(position, Some(glam::Vec3::Y), "field not updated in place");
}

/// A component removed on the project's side leaves the mirror too —
/// both a reflected one and a parked one.
#[test]
fn reapply_drops_components_the_project_removed() {
    let mut resources = ecs();
    let mut mirror = RemoteMirror::new();

    mirror.apply(&snapshot(), &mut resources);
    let mesh = mirror.local_of(eid(1)).expect("mesh mirrored");

    // Both of the mesh's components are gone on the next pull.
    let mut next = snapshot();
    next[1].components.clear();
    mirror.apply(&next, &mut resources);

    assert!(
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Transform>())
            .and_then(|s| s.get(mesh))
            .is_none(),
        "reflected component survived"
    );
    assert_eq!(
        resources
            .get::<DynamicComponents>()
            .unwrap()
            .iter_entity(mesh)
            .count(),
        0,
        "parked component survived"
    );
}
