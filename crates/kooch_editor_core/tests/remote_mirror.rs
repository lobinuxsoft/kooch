//! `RemoteMirror` reconstructs a remote snapshot into a local ECS,
//! keyed by entity id, parking unknown project components.

use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::dynamic_components::DynamicComponents;
use kooch_ecs::hierarchy::Parent;
use kooch_ecs::query::AccessTracker;
use kooch_ecs::reflect::ReflectValue;
use kooch_ecs::transform::Transform;

use kooch_editor_core::remote_mirror::RemoteMirror;
use kooch_remote::protocol::{ComponentSnapshot, EntityId, EntitySnapshot};

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

/// Unparenting on the project has to reach the mirror.
///
/// The parent travels as its own snapshot field rather than as a component,
/// so `sync_components` never sees it and cannot retire it. The pass that
/// wires parents used to skip entities whose snapshot reported no parent,
/// which meant *parenting* was applied and *unparenting* was ignored — the
/// hierarchy stayed nested in the editor while the project had already
/// flattened it. Asymmetric, and it read as "unparent does not work".
#[test]
fn a_vanished_parent_is_cleared_from_the_mirror() {
    let mut resources = ecs();
    let mut mirror = RemoteMirror::new();

    mirror.apply(&snapshot(), &mut resources);
    let mesh = mirror.local_of(eid(1)).expect("mesh mirrored");

    // The fixture nests the mesh under the rig.
    assert!(
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Parent>())
            .is_some_and(|s| s.contains(mesh)),
        "the fixture is not exercising a parented entity"
    );

    // The project flattens it.
    let mut next = snapshot();
    next[1].parent = None;
    mirror.apply(&next, &mut resources);

    assert!(
        resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Parent>())
            .is_none_or(|s| !s.contains(mesh)),
        "the mirror kept a Parent the project no longer reports"
    );
}

/// Re-parenting to a *different* entity travels too, not just to root.
#[test]
fn a_changed_parent_is_followed() {
    let mut resources = ecs();
    let mut mirror = RemoteMirror::new();

    // Three entities: the third starts as a root and becomes the new parent.
    let mut initial = snapshot();
    initial.push(EntitySnapshot {
        id: eid(2),
        name: Some("Other".into()),
        parent: None,
        components: vec![],
    });
    mirror.apply(&initial, &mut resources);

    let mesh = mirror.local_of(eid(1)).expect("mesh mirrored");
    let other = mirror.local_of(eid(2)).expect("other mirrored");

    let mut next = initial.clone();
    next[1].parent = Some(eid(2));
    mirror.apply(&next, &mut resources);

    let parent = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Parent>())
        .and_then(|s| s.get(mesh))
        .map(|p| p.entity);
    assert_eq!(
        parent,
        Some(other),
        "the mirror did not follow the new parent"
    );
}

/// A component pointing at another entity has to point at the *mirror's*
/// entity, not at the project's handle.
///
/// The two processes number their entities independently, so index 9 over
/// there is not index 9 here. Untranslated, a joint's Body A read as
/// whichever mirror entity happened to land on that index — or as
/// missing, which is how it looked.
#[test]
fn an_entity_reference_is_translated_into_the_mirror() {
    use kooch_ecs::reflect::EntityRef;
    use kooch_physics::components::Joint;

    let mut resources = ecs();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Joint>();
    let mut mirror = RemoteMirror::new();

    // The project's handles are deliberately nothing like the mirror's:
    // high indices no freshly-spawned local entity would be given.
    let target = EntityId {
        index: 900,
        generation: 3,
    };
    let holder = EntityId {
        index: 901,
        generation: 3,
    };
    let snapshot = vec![
        EntitySnapshot {
            id: target,
            name: Some("Door frame".into()),
            parent: None,
            components: vec![],
        },
        EntitySnapshot {
            id: holder,
            name: Some("Hinge".into()),
            parent: None,
            components: vec![ComponentSnapshot {
                type_name: std::any::type_name::<Joint>().into(),
                fields: vec![(
                    "body_a".into(),
                    ReflectValue::EntityRef(Some(EntityRef::live(target.into()))),
                )],
            }],
        },
    ];

    mirror.apply(&snapshot, &mut resources);

    let local_target = mirror.local_of(target).expect("the target is mirrored");
    let local_holder = mirror.local_of(holder).expect("the holder is mirrored");
    let joint = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Joint>())
        .and_then(|s| s.get(local_holder).copied())
        .expect("the Joint was mirrored");

    assert_eq!(
        joint.body_a,
        Some(EntityRef::live(local_target)),
        "the reference still names the project's handle",
    );
}

/// The reference is translated even when its target appears *after* the
/// entity holding it — snapshot order is archetype order, not authoring
/// order, so this is the ordinary case rather than a corner.
#[test]
fn a_reference_to_a_later_entity_is_translated_too() {
    use kooch_ecs::reflect::EntityRef;
    use kooch_physics::components::Joint;

    let mut resources = ecs();
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .register_cpu_reflected::<Joint>();
    let mut mirror = RemoteMirror::new();

    let snapshot = vec![
        EntitySnapshot {
            id: eid(5),
            name: Some("Hinge".into()),
            parent: None,
            components: vec![ComponentSnapshot {
                type_name: std::any::type_name::<Joint>().into(),
                fields: vec![(
                    "body_b".into(),
                    ReflectValue::EntityRef(Some(EntityRef::live(eid(6).into()))),
                )],
            }],
        },
        EntitySnapshot {
            id: eid(6),
            name: Some("Door".into()),
            parent: None,
            components: vec![],
        },
    ];

    mirror.apply(&snapshot, &mut resources);

    let local_target = mirror.local_of(eid(6)).expect("the target is mirrored");
    let joint = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Joint>())
        .and_then(|s| s.get(mirror.local_of(eid(5)).unwrap()).copied())
        .expect("the Joint was mirrored");

    assert_eq!(joint.body_b, Some(EntityRef::live(local_target)));
}
