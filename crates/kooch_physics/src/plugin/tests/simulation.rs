//! What the solver does once it is running: gravity, the play gate,
//! frame-rate independence, and who owns a pose.

use super::*;

/// Acceptance: "An entity with `PhysicsBody` + `Collider` falls under
/// gravity when Play is pressed."
#[test]
fn a_dynamic_body_falls_while_playing() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);
    Playing::set(&mut resources, true);

    simulate(&mut resources, 30);

    let y = position(&resources, entity).y;
    assert!(y < 9.9, "the body did not fall: y = {y}");
}

/// Acceptance: "The simulation does not advance while paused."
#[test]
fn the_simulation_is_inert_while_authoring() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);

    simulate(&mut resources, 30);

    assert_eq!(
        position(&resources, entity),
        Vec3::new(0.0, 10.0, 0.0),
        "the world moved while nobody pressed play"
    );
}

/// Acceptance: "Stepping is frame-rate independent: same result at 30 and
/// 144 fps." The step size comes from `Time::fixed_delta`, so a slow frame
/// runs more steps rather than one bigger step — a second of simulated
/// time is a second either way.
#[test]
fn stepping_is_frame_rate_independent() {
    fn fall_after_one_second(hz: f64) -> f32 {
        let mut resources = world();
        let entity = falling_sphere(&mut resources, 100.0);
        resources.get_mut::<Time>().unwrap().set_fixed_hz(hz);
        Playing::set(&mut resources, true);

        simulate(&mut resources, hz as u32);
        100.0 - position(&resources, entity).y
    }

    let at_30 = fall_after_one_second(30.0);
    let at_144 = fall_after_one_second(144.0);

    // Semi-implicit Euler leaves a step-size-dependent residue, so this is
    // "the same fall", not "bit-identical" — that is #568's problem.
    assert!(
        (at_30 - at_144).abs() < 0.25,
        "one second of gravity differs by frame rate: {at_30} vs {at_144}"
    );
    assert!(at_144 > 4.0, "gravity barely moved it: {at_144}");
}

/// A static body is never moved by the solver, whatever lands on it.
#[test]
fn a_static_body_stays_put_and_catches_what_falls() {
    let mut resources = world();
    let floor = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        PhysicsBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(10.0, 0.5, 10.0),
            ..Default::default()
        },
    );
    let sphere = falling_sphere(&mut resources, 5.0);
    Playing::set(&mut resources, true);

    simulate(&mut resources, 180);

    assert_eq!(position(&resources, floor), Vec3::ZERO);
    let resting = position(&resources, sphere).y;
    assert!(
        (0.5..1.5).contains(&resting),
        "the sphere did not come to rest on the floor: y = {resting}"
    );
}

/// Acceptance: "Kinematic bodies follow `Transform`." The authored pose
/// drives the body even while playing — the opposite direction from a
/// dynamic body.
#[test]
fn a_kinematic_body_follows_its_transform_while_playing() {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        PhysicsBody {
            kind: KIND_KINEMATIC,
            mass: 1.0,
            ..Default::default()
        },
        Collider::default(),
    );
    Playing::set(&mut resources, true);
    simulate(&mut resources, 1);

    insert(
        &mut resources,
        entity,
        Transform::from_position(Vec3::new(3.0, 0.0, 0.0)),
    );
    simulate(&mut resources, 2);

    assert!(
        solver_position(&resources, entity).x > 2.9,
        "the solver ignored the authored pose: {}",
        solver_position(&resources, entity)
    );
    // And writeback left the authored value alone.
    assert_eq!(position(&resources, entity).x, 3.0);
}

/// Dragging a gizmo while authoring has to move the collider, or the next
/// Play starts from a world the solver has never seen.
#[test]
fn authoring_a_transform_moves_the_body() {
    let mut resources = world();
    let entity = falling_sphere(&mut resources, 10.0);
    physics_sync_system(&mut resources);

    insert(
        &mut resources,
        entity,
        Transform::from_position(Vec3::new(0.0, 42.0, 0.0)),
    );
    physics_sync_system(&mut resources);

    assert_eq!(solver_position(&resources, entity).y, 42.0);
}

/// The offset has to reach the solver, not just the gizmo. A capsule
/// pushed up by half a body — the character-pivoted-at-the-feet case —
/// rests higher off the floor than one centred on its entity.
#[test]
fn an_offset_shape_collides_where_it_is_drawn() {
    fn rest_height(center: Vec3) -> f32 {
        let mut resources = world();
        spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, -0.5, 0.0)),
            PhysicsBody {
                kind: KIND_STATIC,
                mass: 0.0,
                ..Default::default()
            },
            Collider {
                shape: SHAPE_CUBOID,
                half_extents: Vec3::new(20.0, 0.5, 20.0),
                ..Default::default()
            },
        );
        let ball = spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, 6.0, 0.0)),
            PhysicsBody::default(),
            Collider {
                center,
                ..Default::default()
            },
        );
        Playing::set(&mut resources, true);
        simulate(&mut resources, 300);
        position(&resources, ball).y
    }

    // Floor top at y = 0, sphere radius 0.5.
    let centred = rest_height(Vec3::ZERO);
    // The shape sits 2 units above the body's origin, so the *body* comes
    // to rest 2 units lower for the shape to touch the same floor.
    let offset = rest_height(Vec3::new(0.0, 2.0, 0.0));

    assert!(
        (centred - 0.5).abs() < 0.15,
        "the centred sphere did not rest on the floor: y = {centred}"
    );
    assert!(
        (offset - (centred - 2.0)).abs() < 0.2,
        "the offset never reached the solver: centred {centred}, offset {offset}"
    );
}

/// The offset moves the shape, never the body. Writeback still reports
/// where the *body* is, or every transform in the scene would drift by the
/// collider's offset every frame.
#[test]
fn an_offset_does_not_move_the_body() {
    let mut resources = world();
    let entity = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::new(1.0, 5.0, -2.0)),
        PhysicsBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            center: Vec3::new(0.0, 10.0, 0.0),
            ..Default::default()
        },
    );
    Playing::set(&mut resources, true);
    simulate(&mut resources, 10);

    assert_eq!(
        position(&resources, entity),
        Vec3::new(1.0, 5.0, -2.0),
        "the collider's offset leaked into the body's transform"
    );
}
