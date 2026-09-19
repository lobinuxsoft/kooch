//! Bodies under fields: sleep and waking, planes, priority zones, suppression, dominance, scaling.

use super::*;

/// A resting body must be allowed to sleep: a field waking every body every step would simulate
/// settled crates forever, which the world vector never did.
#[test]
fn a_settled_body_still_falls_asleep() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::default(),
        GlobalGravity::default(),
    );

    // A floor to settle on, and something to settle.
    let floor = spawn(&mut resources);
    insert(&mut resources, floor, Transform::default());
    insert(
        &mut resources,
        floor,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        floor,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(20.0, 0.5, 20.0),
            ..Default::default()
        },
    );

    let crate_body = body_at(&mut resources, Vec3::new(0.0, 1.2, 0.0));

    Playing::set(&mut resources, true);
    // Long enough to fall, bounce out its energy, and cross rapier's sleep
    // timer, which is a wall-clock threshold rather than a step count.
    simulate(&mut resources, 600);

    let handle = resources
        .get::<PhysicsWorld>()
        .and_then(|w| w.iter().find(|(_, e, _, _)| *e == crate_body).map(|t| t.3))
        .expect("the crate has no body");
    let sleeping = resources
        .get::<PhysicsWorld>()
        .and_then(|w| w.backend().is_sleeping(handle))
        .expect("stale handle");

    assert!(
        sleeping,
        "the crate is still awake after ten seconds of resting on a floor \
         — the field is waking every body every step, so nothing in the \
         scene ever sleeps",
    );
}

/// …but a field that changes has to wake what it pulls on — only on the step that sees the change.
#[test]
fn a_moved_source_wakes_what_it_pulls_on() {
    let mut resources = world();
    let planet = source_at(
        &mut resources,
        Transform::default(),
        GlobalGravity::default(),
    );

    let floor = spawn(&mut resources);
    insert(&mut resources, floor, Transform::default());
    insert(
        &mut resources,
        floor,
        PhysicsBody {
            kind: KIND_STATIC,
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        floor,
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(20.0, 0.5, 20.0),
            ..Default::default()
        },
    );

    let crate_body = body_at(&mut resources, Vec3::new(0.0, 1.2, 0.0));
    Playing::set(&mut resources, true);
    simulate(&mut resources, 600);

    let handle = |resources: &Resources| {
        resources
            .get::<PhysicsWorld>()
            .and_then(|w| w.iter().find(|(_, e, _, _)| *e == crate_body).map(|t| t.3))
            .expect("the crate has no body")
    };
    let sleeping = |resources: &Resources| {
        let handle = handle(resources);
        resources
            .get::<PhysicsWorld>()
            .and_then(|w| w.backend().is_sleeping(handle))
            .expect("stale handle")
    };
    assert!(
        sleeping(&resources),
        "it never settled, so the test is moot"
    );

    // Gravity flips upward. A crate that stays asleep through that is a crate glued to the floor.
    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<GlobalGravity>()
        && let Some(field) = storage.get_mut(planet)
    {
        field.acceleration = Vec3::new(0.0, 9.81, 0.0);
    }
    simulate(&mut resources, 2);

    assert!(
        !sleeping(&resources),
        "the field changed and the settled crate slept through it",
    );
}

/// Acceptance for #47: a floor is unbounded across itself, so walking far
/// enough sideways does not walk out of its gravity. An area with large
/// half-extents is the workaround this replaces, and it has an edge.
#[test]
fn a_plane_catches_a_body_far_aside() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        PlaneGravity::default(),
    );
    let body = body_at(&mut resources, Vec3::new(4000.0, 20.0, -4000.0));

    simulate(&mut resources, 30);

    let moved = position(&resources, body);
    assert!(moved.y < 19.0, "should have fallen: {moved}");
    assert!(
        (moved.x - 4000.0).abs() < 1e-2 && (moved.z + 4000.0).abs() < 1e-2,
        "a plane pulls along its normal only: {moved}",
    );
}

/// Acceptance for #47: one-sided. A body under the plane is not dragged back up into it.
#[test]
fn a_plane_ignores_what_is_under_it() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        PlaneGravity::default(),
    );
    assert_eq!(
        plugin::gravity_at(&resources, Vec3::new(0.0, -1.0, 0.0)),
        Vec3::ZERO,
    );
}

/// Acceptance for #48: a room with its own down overrules the planet
/// under it, instead of summing into a diagonal nobody authored.
#[test]
fn a_priority_zone_overrules_the_planet() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    let room = source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        AreaGravity {
            direction: Vec3::X,
            ..Default::default()
        },
    );
    insert(&mut resources, room, GravityPriority { level: 1 });

    let inside = plugin::gravity_at(&resources, Vec3::ZERO);
    assert!(
        (inside - Vec3::new(9.81, 0.0, 0.0)).length() < 1e-3,
        "the planet should be gone inside the room: {inside}",
    );
}

/// Acceptance for #48: the override is proportional to the zone's own
/// reach, so a body crossing the boundary is not snapped.
#[test]
fn suppression_fades_with_the_zone() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    let room = source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        AreaGravity {
            direction: Vec3::X,
            half_extents: Vec3::splat(5.0),
            falloff: 10.0,
            ..Default::default()
        },
    );
    insert(&mut resources, room, GravityPriority { level: 1 });

    // Five metres outside a ten-metre fade: the room is at half strength,
    // so half the planet is back.
    let edge = plugin::gravity_at(&resources, Vec3::new(10.0, 0.0, 0.0));
    assert!(
        (edge - Vec3::new(4.905, -4.905, 0.0)).length() < 1e-3,
        "half the room and half the planet: {edge}",
    );
}

/// Equal levels sum, exactly as they did before priorities existed.
#[test]
fn equal_levels_still_sum() {
    let mut resources = world();
    for _ in 0..2 {
        let source = source_at(
            &mut resources,
            Transform::from_position(Vec3::ZERO),
            GlobalGravity::default(),
        );
        insert(&mut resources, source, GravityPriority { level: 3 });
    }
    let total = plugin::gravity_at(&resources, Vec3::ZERO);
    assert!((total.y + 19.62).abs() < 1e-3, "{total}");
}

/// Acceptance for #48: "dominant gravity" for orientation. The sum points
/// between two planets, which is correct and reads as a character standing
/// at a slant; the dominant one snaps to whichever is winning.
#[test]
fn the_dominant_source_ignores_the_weaker() {
    let mut resources = world();
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        PointGravity::default(),
    );
    source_at(
        &mut resources,
        Transform::from_position(Vec3::new(50.0, 50.0, 0.0)),
        PointGravity {
            strength: 3.0,
            ..Default::default()
        },
    );
    let point = Vec3::new(0.0, 50.0, 0.0);

    let summed = plugin::gravity_up(&resources, point);
    assert!(
        summed.x.abs() > 0.1,
        "the sum leans towards the second: {summed}"
    );

    let dominant = plugin::gravity_dominant(&resources, point);
    assert!(
        (dominant - Vec3::Y).length() < 1e-3,
        "up is away from the stronger planet: {dominant}",
    );
}

/// A field's space is rigid, so its extents are metres; a gizmo test alone pins only what is drawn.
#[test]
fn scaling_a_source_does_not_resize_it() {
    fn pull_at(scale: f32, height: f32) -> f32 {
        let mut resources = world();
        source_at(
            &mut resources,
            Transform {
                position: Vec3::ZERO,
                rotation: glam::Quat::IDENTITY,
                scale: Vec3::splat(scale),
            },
            BoxGravity::default(),
        );
        plugin::gravity_at(&resources, Vec3::new(0.0, height, 0.0)).length()
    }

    // Inside the solid, in the band, and past the fade — the three answers a box source has.
    for height in [2.0, 20.0, 100.0] {
        assert!(
            (pull_at(1.0, height) - pull_at(8.0, height)).abs() < 1e-4,
            "a scale of 8 changed the pull at {height} m",
        );
    }
}
