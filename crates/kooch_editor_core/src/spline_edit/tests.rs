use super::*;
use glam::Vec3;

fn listed(knots: &[Knot]) -> ReflectValue {
    list_value(knots)
}

fn knots_of(value: &ReflectValue) -> Vec<Knot> {
    list_from(value.clone(), None, "points").expect("a list of knots")
}

/// 🔴 The point of the whole module: an added knot lands ahead of the one before it instead of on
/// the origin, where a reflected list's fixed `element` would always put it.
#[test]
fn an_added_knot_lands_ahead() {
    let before = [
        Knot::at(Vec3::new(5.0, 0.0, 0.0)),
        Knot::at(Vec3::new(6.0, 0.0, 0.0)),
        Knot::default(),
    ];
    let after = knots_of(&placed(&listed(&before)).expect("placed"));

    assert_ne!(after[2].position, Vec3::ZERO, "still on the origin");
    // Ahead of the previous knot, by the step the author was already using.
    assert!(
        after[2].position.distance(before[1].position) > 0.5,
        "{}",
        after[2].position
    );
    assert!(after[2].position.x > before[1].position.x, "went backwards");
}

/// The rhythm is copied, so a path built at five-metre steps keeps them.
#[test]
fn the_spacing_is_copied() {
    let before = [
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(5.0, 0.0, 0.0)),
        Knot::default(),
    ];
    let after = knots_of(&placed(&listed(&before)).expect("placed"));

    assert!(
        (after[2].position.distance(before[1].position) - 5.0).abs() < 0.1,
        "{}",
        after[2].position
    );
}

/// The first knot belongs at the origin it was added at: there is nothing to be ahead of.
#[test]
fn the_first_knot_is_left_alone() {
    assert!(placed(&listed(&[Knot::default()])).is_none());
    assert!(placed(&listed(&[])).is_none());
}

/// A knot the author already moved is not a new one, and moving it would undo their edit.
#[test]
fn a_placed_knot_is_not_moved_again() {
    let settled = [
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(1.0, 0.0, 0.0)),
        Knot::at(Vec3::new(2.0, 0.0, 0.0)),
    ];
    assert!(placed(&listed(&settled)).is_none());
}

/// Only the LAST knot is the added one. A default in the middle is a knot the author put there.
#[test]
fn a_default_in_the_middle_stays() {
    let middle = [
        Knot::at(Vec3::new(1.0, 0.0, 0.0)),
        Knot::default(),
        Knot::at(Vec3::new(3.0, 0.0, 0.0)),
    ];
    assert!(placed(&listed(&middle)).is_none());
}

/// Coincident knots give a zero step, and the fallback keeps the new one off the previous one.
#[test]
fn coincident_knots_still_step() {
    let stalled = [Knot::at(Vec3::ZERO), Knot::at(Vec3::ZERO), Knot::default()];
    let after = knots_of(&placed(&listed(&stalled)).expect("placed"));

    assert!(after[2].position.length() > 0.5, "{}", after[2].position);
    assert!(after[2].position.is_finite());
}

/// 🔴 The placement has to run where the edit is DISPATCHED, and the first version did not: it sat
/// after the local/dynamic split and compared `TypeId`, so with a project connected — where a
/// mirrored world resolves components by name — it never ran at all. Every added knot kept landing
/// on the origin, and the unit tests above could not see it because the function itself was right.
#[test]
fn the_dispatch_places_an_added_knot() {
    use kooch_core::resource::Resources;
    use kooch_ecs::allocator::EntityAllocator;
    use kooch_ecs::archetype_registry::ArchetypeRegistry;
    use kooch_ecs::commands::Commands;
    use kooch_ecs::component::{ComponentNames, ComponentRegistry};
    use kooch_ecs::dynamic_components::DynamicComponents;
    use kooch_ecs::query::AccessTracker;
    use kooch_ecs::spline::Spline;

    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let entity = allocator.spawn();
    resources.insert(allocator);
    let mut archetypes = ArchetypeRegistry::new();
    let empty = archetypes.get_or_create(Default::default());
    archetypes.register_entity(entity, empty);
    resources.insert(archetypes);
    resources.insert(AccessTracker::new());
    resources.insert(Commands::new());
    resources.insert(DynamicComponents::new());
    resources.insert(ComponentNames::new());
    resources.insert(crate::undo::UndoStack::new());

    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Spline>();
    if let Some(storage) = registry.get_cpu_mut::<Spline>() {
        storage.insert(
            entity,
            Spline {
                points: vec![Knot::at(Vec3::ZERO), Knot::at(Vec3::new(3.0, 0.0, 0.0))],
                closed: false,
            },
        );
    }
    resources.insert(registry);
    crate::queries::intern_registry_names(&mut resources);

    let component = resources
        .get::<ComponentNames>()
        .and_then(|names| names.id(std::any::type_name::<Spline>()))
        .expect("Spline is interned");

    // What the Inspector's add button produces: the list with one more `Knot::default()`.
    let added = listed(&[
        Knot::at(Vec3::ZERO),
        Knot::at(Vec3::new(3.0, 0.0, 0.0)),
        Knot::default(),
    ]);
    let mut undo = resources.remove::<crate::undo::UndoStack>().unwrap();
    crate::actions::apply_actions(
        &mut resources,
        &[crate::actions::EditorAction::SetField {
            entity,
            component,
            field: "points".into(),
            value: added,
        }],
        &mut undo,
    );
    resources.insert(undo);

    let stored = resources
        .get::<ComponentRegistry>()
        .and_then(|registry| registry.get_cpu::<Spline>()?.get(entity).cloned())
        .expect("the spline is still there");
    assert_eq!(stored.points.len(), 3);
    assert_ne!(
        stored.points[2].position,
        Vec3::ZERO,
        "the added knot was left on the origin — the dispatch hook did not run"
    );
}
