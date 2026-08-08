use super::*;
use kooch_ecs::reflect::ReflectValue;
use kooch_remote::protocol::ComponentSnapshot;

fn entity(index: u32, x: f32) -> EntitySnapshot {
    EntitySnapshot {
        id: EntityId {
            index,
            generation: 0,
        },
        name: Some(format!("Entity {index}")),
        parent: None,
        components: vec![ComponentSnapshot {
            type_name: "Transform".to_owned(),
            fields: vec![("x".to_owned(), ReflectValue::F32(x))],
        }],
    }
}

#[test]
fn a_changed_entity_replaces_its_previous_value() {
    let mut world = vec![entity(0, 1.0), entity(1, 2.0)];
    merge_into(&mut world, vec![entity(1, 9.0)], &[]);

    assert_eq!(world.len(), 2);
    let moved = &world[1].components[0].fields[0].1;
    assert_eq!(*moved, ReflectValue::F32(9.0));
}

#[test]
fn a_new_entity_is_added() {
    let mut world = vec![entity(0, 1.0)];
    merge_into(&mut world, vec![entity(5, 3.0)], &[]);
    assert_eq!(world.len(), 2);
}

/// A despawn only travels as an id. Miss it and the mirror shows an
/// entity the project deleted, editable and going nowhere.
#[test]
fn a_removed_entity_disappears() {
    let mut world = vec![entity(0, 1.0), entity(1, 2.0)];
    merge_into(
        &mut world,
        vec![],
        &[EntityId {
            index: 1,
            generation: 0,
        }],
    );

    assert_eq!(world.len(), 1);
    assert_eq!(world[0].id.index, 0);
}

/// Removals are applied before additions. An index despawned and
/// reused inside one revision arrives as both, and the wrong order
/// would delete the entity that had just been added.
#[test]
fn an_index_removed_and_re_added_survives() {
    let mut world = vec![entity(0, 1.0), entity(1, 2.0)];
    merge_into(
        &mut world,
        vec![entity(1, 7.0)],
        &[EntityId {
            index: 1,
            generation: 0,
        }],
    );

    assert_eq!(world.len(), 2, "the re-added entity was dropped");
    let value = &world[1].components[0].fields[0].1;
    assert_eq!(*value, ReflectValue::F32(7.0));
}

/// Downstream reads the snapshot as authored order, which is
/// ascending index. Appending would put every new entity last.
#[test]
fn the_world_stays_sorted_by_index() {
    let mut world = vec![entity(0, 1.0), entity(9, 1.0)];
    merge_into(&mut world, vec![entity(4, 1.0)], &[]);

    let order: Vec<u32> = world.iter().map(|e| e.id.index).collect();
    assert_eq!(order, vec![0, 4, 9]);
}

/// The common case, and the reason the feature exists: a world that
/// did not move is left exactly as it was.
#[test]
fn an_empty_delta_changes_nothing() {
    let world = vec![entity(0, 1.0), entity(1, 2.0)];
    let mut world = world.clone();
    merge_into(&mut world, vec![], &[]);
    assert_eq!(world, world);
}
