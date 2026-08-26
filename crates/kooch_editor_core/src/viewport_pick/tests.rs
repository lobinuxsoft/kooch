use super::*;

/// `Authored` has nothing to resolve, and must not accidentally become
/// a position — that would move every prefab dropped in the World panel.
#[test]
fn an_authored_drop_resolves_to_nothing() {
    let resources = Resources::new();
    assert_eq!(resolve(&resources, DropPoint::Authored), None);
}

/// A queryable world with nothing in it. `Query::new` requires the
/// registries — a `Resources` without them is not an empty world, it is
/// not a world — so this is what "no camera" actually looks like.
fn empty_world() -> Resources {
    let mut resources = Resources::new();
    resources.insert(kooch_ecs::component::ComponentRegistry::new());
    resources.insert(kooch_ecs::archetype_registry::ArchetypeRegistry::new());
    resources.insert(kooch_ecs::query::AccessTracker::new());
    resources
}

/// With no camera there is nothing to unproject against. Returning a
/// position anyway would place the object at a made-up point.
#[test]
fn a_viewport_drop_with_no_camera_resolves_to_nothing() {
    let point = DropPoint::Viewport {
        cursor: Vec2::new(10.0, 10.0),
        viewport_size: Vec2::new(800.0, 600.0),
    };
    assert_eq!(resolve(&empty_world(), point), None);
}
