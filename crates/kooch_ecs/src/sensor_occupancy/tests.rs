use super::*;

fn entity(index: u32) -> Entity {
    Entity::new(index, 0)
}

#[test]
fn a_body_stays_until_it_leaves() {
    let mut inside = SensorOccupancy::default();
    inside.enter(entity(1), entity(2), 0.5);
    assert_eq!(inside.depth_in(entity(1)), Some(0.5));
    inside.leave(entity(1), entity(2));
    assert!(inside.is_empty());
}

/// Arriving twice is one occupant at the newer depth — the solver reports an arrival per frame a
/// pair begins touching, and a reload can repeat one.
#[test]
fn a_second_arrival_moves_it() {
    let mut inside = SensorOccupancy::default();
    inside.enter(entity(1), entity(2), 0.5);
    inside.enter(entity(1), entity(2), 2.0);
    assert_eq!(inside.iter().count(), 1);
    assert_eq!(inside.depth_in(entity(1)), Some(2.0));
}

/// 🔴 Several bodies in one region: the deepest answers for it. Taking the first would make the
/// effect flicker as bodies are added and removed around whoever is actually inside.
#[test]
fn the_deepest_body_answers() {
    let mut inside = SensorOccupancy::default();
    inside.enter(entity(1), entity(2), 0.25);
    inside.enter(entity(1), entity(3), 3.0);
    assert_eq!(inside.depth_in(entity(1)), Some(3.0));
}

/// A departure that never arrived is a scene reload, not a panic.
#[test]
fn an_unknown_departure_is_quiet() {
    let mut inside = SensorOccupancy::default();
    inside.leave(entity(1), entity(2));
    assert!(inside.is_empty());
}

#[test]
fn a_deleted_entity_is_forgotten() {
    let mut inside = SensorOccupancy::default();
    inside.enter(entity(1), entity(2), 1.0);
    inside.enter(entity(3), entity(1), 1.0);
    inside.forget(entity(1));
    assert!(inside.is_empty(), "both sides of it should go");
}
