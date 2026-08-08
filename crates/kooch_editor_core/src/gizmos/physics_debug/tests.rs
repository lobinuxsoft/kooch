use super::*;
use glam::Vec3;
use kooch_physics::backend::{BodyDesc, CollisionShape, PhysicsBackend};
use kooch_physics::rapier_backend::RapierBackend;

fn world_with_a_body() -> Resources {
    let mut backend = RapierBackend::new();
    backend.add_body(BodyDesc::dynamic(
        CollisionShape::Sphere { radius: 0.5 },
        1.0,
    ));
    let mut resources = Resources::new();
    resources.insert(PhysicsWorld::new(Box::new(backend)));
    resources
}

fn line_count(batch: &GizmoBatch) -> usize {
    batch.lines.len()
}

/// A host that never inserted the resource must not pay for the
/// overlay, or every headless tool walks the physics world for nothing.
#[test]
fn without_the_resource_nothing_is_drawn() {
    let mut resources = world_with_a_body();
    let mut batch = GizmoBatch::default();

    draw(&mut resources, &mut batch);

    assert_eq!(line_count(&batch), 0);
}

/// The default is every switch down, and that has to mean the walk
/// never happens rather than happening and drawing nothing.
#[test]
fn the_overlay_is_off_by_default() {
    let mut resources = world_with_a_body();
    resources.insert(PhysicsDebugOverlay::default());
    let mut batch = GizmoBatch::default();

    draw(&mut resources, &mut batch);

    assert_eq!(line_count(&batch), 0);
    assert!(!resources.get::<PhysicsDebugOverlay>().unwrap().is_active());
}

#[test]
fn switching_a_category_on_draws_geometry() {
    let mut resources = world_with_a_body();
    resources.insert(PhysicsDebugOverlay {
        categories: DebugCategories {
            collider_shapes: true,
            ..Default::default()
        },
        ..Default::default()
    });
    let mut batch = GizmoBatch::default();

    draw(&mut resources, &mut batch);

    assert!(line_count(&batch) > 0, "the overlay drew nothing");
}

/// The resource has to go back, or the overlay works for exactly one
/// frame and then silently turns itself off.
#[test]
fn the_overlay_survives_a_frame() {
    let mut resources = world_with_a_body();
    resources.insert(PhysicsDebugOverlay {
        categories: DebugCategories::all(),
        ..Default::default()
    });
    let mut batch = GizmoBatch::default();

    draw(&mut resources, &mut batch);
    let first = line_count(&batch);
    batch.clear();
    draw(&mut resources, &mut batch);

    assert!(resources.get::<PhysicsDebugOverlay>().is_some());
    assert_eq!(line_count(&batch), first, "the second frame differs");
}

/// The scratch buffer is reused, so it has to be cleared or the
/// overlay grows without bound while it is on.
#[test]
fn the_scratch_buffer_does_not_accumulate() {
    let mut resources = world_with_a_body();
    resources.insert(PhysicsDebugOverlay {
        categories: DebugCategories::all(),
        ..Default::default()
    });
    let mut batch = GizmoBatch::default();

    draw(&mut resources, &mut batch);
    let first = resources
        .get::<PhysicsDebugOverlay>()
        .unwrap()
        .scratch
        .len();
    draw(&mut resources, &mut batch);
    let second = resources
        .get::<PhysicsDebugOverlay>()
        .unwrap()
        .scratch
        .len();

    assert_eq!(first, second, "the buffer grew between frames");
}

/// Sanity on the whole path: an empty world has nothing to say even
/// with everything switched on.
#[test]
fn an_empty_world_draws_nothing() {
    let mut resources = Resources::new();
    resources.insert(PhysicsWorld::new(Box::new(RapierBackend::new())));
    resources.insert(PhysicsDebugOverlay {
        categories: DebugCategories::all(),
        ..Default::default()
    });
    let mut batch = GizmoBatch::default();

    draw(&mut resources, &mut batch);

    assert_eq!(line_count(&batch), 0);
    let _ = Vec3::ZERO;
}
