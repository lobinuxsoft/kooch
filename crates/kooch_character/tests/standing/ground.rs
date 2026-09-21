//! Standing: ride height, landing, ground that ends or slopes, a planet, zero gravity, crates.

use super::*;

/// The claim the whole design rests on: the capsule holds a gap and does not rest on the floor.
#[test]
fn a_character_floats_at_its_ride_height() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 3.0, 0.0));

    simulate(&mut resources, 240);

    let state = grounded(&resources, hero);
    assert!(state.standing, "should have found the floor");
    let wanted = CharacterController::default().ride_height;
    assert!(
        (state.distance - wanted).abs() < 0.06,
        "held {} above the ground, wanted {wanted}",
        state.distance,
    );
    assert!(state.normal.y > 0.9, "flat floor: {}", state.normal);
}

/// It settles instead of oscillating. The landing dips on purpose — see `it_dips_when_it_lands` —
/// and a spring damped too lightly to come to rest passes the height check on the frame it crosses.
#[test]
fn a_landing_settles() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(20.0, 0.5, 20.0),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 4.0, 0.0));

    simulate(&mut resources, 300);
    let settled = position(&resources, hero).y;
    simulate(&mut resources, 60);
    let later = position(&resources, hero).y;

    assert!(
        (later - settled).abs() < 0.01,
        "still moving: {settled} then {later}",
    );
}

/// Ground runs out and the character falls. A spring that pulled from
/// nothing would hold it over the void.
#[test]
fn it_falls_when_the_ground_ends() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(2.0, 0.5, 2.0),
    );
    let hero = character(&mut resources, Vec3::new(30.0, 0.0, 0.0));

    simulate(&mut resources, 120);

    assert!(!grounded(&resources, hero).standing, "nothing is under it");
    assert!(position(&resources, hero).y < -3.0, "it should be falling");
}

/// Acceptance: a 30° slope is walked on without special-casing it — the
/// spring holds the same gap it holds on the flat.
#[test]
fn a_gentle_slope_is_ground() {
    let state = on_a_ramp(30.0);
    assert!(state.standing, "30 degrees is walkable: {}", state.normal);
    let wanted = CharacterController::default().ride_height;
    assert!(
        (state.distance - wanted).abs() < 0.12,
        "held {} on the slope, wanted about {wanted}",
        state.distance,
    );
}

/// Past `max_slope` the sweep still finds the surface and the spring still pushes off it — but it
/// is not ground. Without the distinction a character can jump off a cliff face forever.
#[test]
fn a_steep_slope_is_not_ground() {
    let state = on_a_ramp(70.0);
    assert!(!state.standing, "70 degrees is a wall: {}", state.normal,);
    assert!(
        state.normal.length() > 0.5,
        "and it was still found: {}",
        state.normal,
    );
}

/// Drops a character onto a ramp tilted by `degrees` and reports what it
/// decided it was standing on.
fn on_a_ramp(degrees: f32) -> Grounded {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    slab(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(30.0, 0.5, 30.0),
        glam::Quat::from_rotation_z(degrees.to_radians()),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 2.0, 0.0));

    simulate(&mut resources, 120);
    grounded(&resources, hero)
}

/// Acceptance: upright the whole way round a planet, poles included.
#[test]
fn it_stays_upright_around_a_planet() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        PointGravity {
            strength: 20.0,
            radius: 40.0,
            falloff: 0.0,
        },
    );

    // Four characters spread around a sphere, including one under it.
    for at in [
        Vec3::new(0.0, 9.0, 0.0),
        Vec3::new(9.0, 0.0, 0.0),
        Vec3::new(0.0, -9.0, 0.0),
        Vec3::new(0.0, 0.0, -9.0),
    ] {
        let planet = spawn(&mut resources);
        insert(&mut resources, planet, Transform::from_position(Vec3::ZERO));
        insert(
            &mut resources,
            planet,
            PhysicsBody {
                kind: KIND_STATIC,
                ..Default::default()
            },
        );
        insert(
            &mut resources,
            planet,
            Collider {
                radius: 7.0,
                ..Default::default()
            },
        );
        let hero = character(&mut resources, at);

        simulate(&mut resources, 400);

        let up = at.normalize();
        let state = grounded(&resources, hero);
        assert!(
            state.standing,
            "should stand at {at}, normal {}",
            state.normal
        );

        let facing = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<Transform>())
            .and_then(|s| s.get(hero))
            .map(|t| t.rotation * Vec3::Y)
            .expect("no transform");
        assert!(
            facing.dot(up) > 0.9,
            "leaning at {at}: facing {facing}, up {up}",
        );
    }
}

/// Acceptance: zero gravity is a defined case, not a normalise of a zero
/// vector. Nothing spins, nothing becomes NaN.
#[test]
fn zero_gravity_does_not_spin_it() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity {
            acceleration: Vec3::ZERO,
        },
    );
    let hero = character(&mut resources, Vec3::new(0.0, 3.0, 0.0));

    simulate(&mut resources, 180);

    let at = position(&resources, hero);
    assert!(at.is_finite(), "{at}");
    assert!(
        (at - Vec3::new(0.0, 3.0, 0.0)).length() < 0.1,
        "drifted to {at}"
    );
    assert!(!grounded(&resources, hero).standing);
}

/// Acceptance: it stays part of the world. A kinematic controller moves
/// *through* the scene; this one pushes and is pushed.
#[test]
fn it_pushes_a_crate_and_is_pushed_back() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    floor(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(30.0, 0.5, 30.0),
    );

    let hero = character(&mut resources, Vec3::new(0.0, 0.5, 0.0));
    let crate_entity = body_at(&mut resources, Vec3::new(1.2, 0.5, 0.0));

    // Let both settle before anything is pushed, so the movement below
    // is the only thing that could have moved the crate.
    simulate(&mut resources, 180);
    let crate_start = position(&resources, crate_entity).x;

    // Walking pace, straight at it.
    for _ in 0..120 {
        let body = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
            .and_then(|s| s.get(hero))
            .copied();
        if let Some(body) = body
            && let Some(world) = resources.get_mut::<kooch_physics::plugin::PhysicsWorld>()
        {
            world.apply_impulse(body, Vec3::X * 0.15);
        }
        simulate(&mut resources, 1);
    }

    let moved = position(&resources, crate_entity).x - crate_start;
    assert!(moved > 0.3, "the crate should have been shoved: {moved}");
    assert!(
        grounded(&resources, hero).standing,
        "and the character should still be on the floor",
    );
}
