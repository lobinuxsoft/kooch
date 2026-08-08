use kooch_core::resource::Resources;

use crate::commands::Commands;
use crate::name::Name;
use crate::query::AccessTracker;
use crate::transform::Transform;

use super::*;

fn ecs() -> Resources {
    let mut r = Resources::new();
    r.insert(EntityAllocator::new());
    r.insert(ComponentRegistry::new());
    r.insert(ArchetypeRegistry::new());
    r.insert(AccessTracker::new());
    r.insert(Commands::new());
    r.insert(DynamicComponents::new());
    let registry = r.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu_reflected::<Name>();
    registry.register_cpu_reflected::<Transform>();
    r
}

/// Spawns `count` entities carrying a Transform, returning handles.
fn spawn_all(resources: &mut Resources, count: usize) -> Vec<Entity> {
    let mut spawned = Vec::new();
    for _ in 0..count {
        let mut commands = resources.remove::<Commands>().unwrap();
        let entity = commands.spawn(resources).id();
        commands.apply(resources);
        resources.insert(commands);
        if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
            registry.insert_default_reflected(&TypeId::of::<Transform>(), entity);
        }
        add_to_archetype(resources, entity, TypeId::of::<Transform>());
        spawned.push(entity);
    }
    spawned
}

fn position(resources: &Resources, entity: Entity) -> Option<glam::Vec3> {
    resources
        .get::<ComponentRegistry>()?
        .get_cpu::<Transform>()?
        .get(entity)
        .map(|t| t.position)
}

/// The point of the whole type: handles *and* iteration order
/// survive a round-trip, so anything holding an `Entity` still
/// addresses the same thing and systems see the same sequence.
#[test]
fn restore_preserves_entity_handles_and_values() {
    let mut resources = ecs();
    let spawned = spawn_all(&mut resources, 3);

    // Shuffle the world's iteration order away from index order, so
    // the test would fail if restore merely sorted by index.
    resources
        .get_mut::<ArchetypeRegistry>()
        .unwrap()
        .reorder_entities(&[spawned[2], spawned[0], spawned[1]]);
    let expected: Vec<Entity> = vec![spawned[2], spawned[0], spawned[1]];

    let snapshot = WorldSnapshot::capture(&resources);
    assert_eq!(snapshot.len(), 3);

    // Play: move everything and add an entity.
    for &entity in &spawned {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        let _ = registry.reflect_set_field(
            &TypeId::of::<Transform>(),
            entity,
            "position",
            ReflectValue::Vec3(glam::Vec3::splat(7.0)),
        );
    }
    spawn_all(&mut resources, 1);

    snapshot.restore(&mut resources);

    // Same handles — index *and* generation — same order, and the
    // runtime spawn is gone.
    let restored: Vec<Entity> = resources
        .get::<ArchetypeRegistry>()
        .unwrap()
        .iter_matching(&[])
        .flat_map(|a| a.entities().to_vec())
        .collect();
    assert_eq!(restored, expected, "handles or order churned");
    for &entity in &spawned {
        assert_eq!(position(&resources, entity), Some(glam::Vec3::ZERO));
    }
}

/// The allocator comes back too, so the next spawn after a stop gets
/// the handle it would have got if play had never happened.
#[test]
fn restore_rewinds_the_allocator() {
    let mut resources = ecs();
    spawn_all(&mut resources, 2);

    let snapshot = WorldSnapshot::capture(&resources);
    let next_without_play = {
        let mut probe = resources.get::<EntityAllocator>().cloned().unwrap();
        probe.spawn()
    };

    // A play session churns handles: spawn and despawn a few.
    let temps = spawn_all(&mut resources, 3);
    for entity in temps {
        resources
            .get_mut::<EntityAllocator>()
            .unwrap()
            .despawn(entity);
    }

    snapshot.restore(&mut resources);

    let next_after_stop = resources.get_mut::<EntityAllocator>().unwrap().spawn();
    assert_eq!(
        next_after_stop, next_without_play,
        "generations drifted across a play session"
    );
}

/// Components with no local Rust type are parked, and must survive a
/// stop as well — a remote host mirrors a project whose components it
/// cannot instantiate.
#[test]
fn restore_brings_back_parked_components() {
    let mut resources = ecs();
    let entity = spawn_all(&mut resources, 1)[0];
    resources.get_mut::<DynamicComponents>().unwrap().insert(
        entity,
        "game::spin::Spin",
        vec![("rpm".into(), ReflectValue::F32(33.0))],
    );

    let snapshot = WorldSnapshot::capture(&resources);
    resources.get_mut::<DynamicComponents>().unwrap().clear();
    snapshot.restore(&mut resources);

    let parked: Vec<_> = resources
        .get::<DynamicComponents>()
        .unwrap()
        .iter_entity(entity)
        .map(|(name, fields)| (name.to_owned(), fields.to_vec()))
        .collect();
    assert_eq!(parked.len(), 1);
    assert_eq!(parked[0].0, "game::spin::Spin");
    assert_eq!(
        parked[0].1,
        vec![("rpm".to_owned(), ReflectValue::F32(33.0))]
    );
}
