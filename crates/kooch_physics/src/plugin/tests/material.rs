//! Friction, bounce and damping doing what the numbers say.
//!
//! Asserted against each other rather than against absolute distances.
//! What an author needs is "more friction stops it sooner"; the exact
//! stopping distance is a property of rapier's solver and would make these
//! tests fail on a version bump for no reason anyone cares about.

use super::*;

use crate::components::{COMBINE_MAX, COMBINE_MIN, COMBINE_MULTIPLY};

/// A floor with a chosen friction, wide enough that nothing slides off.
fn floor(resources: &mut Resources, friction: f32) -> Entity {
    spawn_body(
        resources,
        Transform::from_position(Vec3::new(0.0, -1.0, 0.0)),
        PhysicsBody {
            kind: KIND_STATIC,
            mass: 0.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::new(200.0, 1.0, 200.0),
            friction,
            ..Default::default()
        },
    )
}

/// A cube resting on the floor, shoved along +X, and where it ends up.
fn slide(friction: f32) -> f32 {
    let mut resources = world();
    floor(&mut resources, friction);
    let cube = spawn_body(
        &mut resources,
        // Exactly on the floor's surface, so the first step is a contact
        // rather than a drop.
        Transform::from_position(Vec3::new(0.0, 0.5, 0.0)),
        PhysicsBody {
            mass: 1.0,
            ..Default::default()
        },
        Collider {
            shape: SHAPE_CUBOID,
            half_extents: Vec3::splat(0.5),
            friction,
            ..Default::default()
        },
    );
    physics_sync_system(&mut resources);
    Playing::set(&mut resources, true);

    // The shove goes in after the body exists, straight into the solver:
    // there is no force component yet (#567), and this test is about
    // friction rather than about how one is applied.
    let handle = resources
        .get::<PhysicsWorld>()
        .and_then(|w| w.handle(slot_of(&resources, cube).expect("no body")))
        .expect("stale");
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        world
            .backend_mut()
            .set_linear_velocity(handle, Vec3::new(8.0, 0.0, 0.0));
    }

    simulate(&mut resources, 180);
    position(&resources, cube).x
}

/// Acceptance: "a low-friction body slides further than a high-friction
/// one."
#[test]
fn less_friction_slides_further() {
    let slippery = slide(0.02);
    let grippy = slide(1.5);

    assert!(
        slippery > grippy,
        "friction 0.02 slid {slippery} m, friction 1.5 slid {grippy} m",
    );
    // And the grippy one actually stopped, rather than both sliding
    // forever with one marginally ahead.
    assert!(
        grippy < slippery / 2.0,
        "friction barely mattered: {grippy} m against {slippery} m",
    );
}

/// Zero friction has to mean zero. A body shoved across a frictionless
/// floor keeps its speed, and anything less means the coefficient is not
/// reaching the solver.
#[test]
fn frictionless_keeps_its_speed() {
    let frictionless = slide(0.0);
    assert!(
        frictionless > 20.0,
        "8 m/s for three seconds on a frictionless floor got {frictionless} m",
    );
}

/// Acceptance: "a restitution of 1 bounces back to roughly its drop
/// height." Asserted as a comparison, because a perfectly elastic bounce
/// is not something any solver returns exactly.
#[test]
fn more_restitution_bounces_higher() {
    fn drop_and_peak(restitution: f32) -> f32 {
        let mut resources = world();
        spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, -1.0, 0.0)),
            PhysicsBody {
                kind: KIND_STATIC,
                mass: 0.0,
                ..Default::default()
            },
            Collider {
                shape: SHAPE_CUBOID,
                half_extents: Vec3::new(20.0, 1.0, 20.0),
                restitution,
                ..Default::default()
            },
        );
        let ball = spawn_body(
            &mut resources,
            Transform::from_position(Vec3::new(0.0, 4.0, 0.0)),
            PhysicsBody {
                mass: 1.0,
                ..Default::default()
            },
            Collider::default(),
        );
        Playing::set(&mut resources, true);

        // The peak *after* the first bounce, which is the quantity
        // restitution describes. Watching the final height would measure
        // how long the test ran.
        let mut lowest = f32::MAX;
        let mut peak_after_bounce = 0.0f32;
        for _ in 0..240 {
            simulate(&mut resources, 1);
            let y = position(&resources, ball).y;
            lowest = lowest.min(y);
            if y > lowest + 1e-3 {
                peak_after_bounce = peak_after_bounce.max(y);
            }
        }
        peak_after_bounce
    }

    let dead = drop_and_peak(0.0);
    let bouncy = drop_and_peak(0.95);

    assert!(
        bouncy > dead + 0.5,
        "restitution 0.95 peaked at {bouncy} m, restitution 0 at {dead} m",
    );
}

/// Acceptance: "angular damping brings a spun body to rest."
///
/// Measured as the angular velocity that survives, which is the quantity
/// damping acts on. Accumulated rotation would be wrong twice over: a
/// quaternion angle wraps, and a body that has already stopped keeps
/// whatever angle it stopped at.
#[test]
fn angular_damping_stops_a_spin() {
    fn spin_left(angular_damping: f32) -> f32 {
        let mut resources = world();
        // No floor and no gravity worth caring about: the body floats, so
        // damping is the only thing acting on its spin.
        let top = spawn_body(
            &mut resources,
            Transform::default(),
            PhysicsBody {
                mass: 1.0,
                angular_damping,
                ..Default::default()
            },
            Collider::default(),
        );
        physics_sync_system(&mut resources);
        Playing::set(&mut resources, true);

        let handle = resources
            .get::<PhysicsWorld>()
            .and_then(|w| w.handle(slot_of(&resources, top).expect("no body")))
            .expect("stale");
        if let Some(world) = resources.get_mut::<PhysicsWorld>() {
            world
                .backend_mut()
                .set_angular_velocity(handle, Vec3::new(0.0, 10.0, 0.0));
        }
        simulate(&mut resources, 120);

        resources
            .get::<PhysicsWorld>()
            .and_then(|w| w.backend().angular_velocity(handle))
            .map(|v| v.length())
            .expect("stale handle")
    }

    let undamped = spin_left(0.0);
    let damped = spin_left(5.0);

    assert!(
        undamped > 9.0,
        "setup: an undamped spin decayed to {undamped} rad/s on its own",
    );
    assert!(
        damped < 1.0,
        "damping 5 left {damped} rad/s after two seconds",
    );
}

/// The coefficients have to reach the solver, and the seam that carries
/// them is the spec — so an edit has to rebuild the body.
#[test]
fn editing_the_friction_rebuilds_the_body() {
    let mut resources = world();
    let cube = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody::default(),
        Collider::default(),
    );
    physics_sync_system(&mut resources);
    let spec = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .spec(slot_of(&resources, cube).expect("no body"));

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Collider>()
        && let Some(collider) = storage.get_mut(cube)
    {
        collider.friction = 1.5;
    }
    physics_sync_system(&mut resources);

    let after = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .spec(slot_of(&resources, cube).expect("no body"));
    assert_ne!(spec, after, "a friction edit never reached the spec");
    assert_eq!(body_count(&resources), 1, "the old body leaked");
}

/// Every rule has to survive the trip to the backend. A dropdown that
/// silently resolves to Average is worse than no dropdown.
#[test]
fn each_combine_rule_reaches_the_material() {
    use crate::backend::CombineRule;

    for (discriminant, expected) in [
        (COMBINE_MIN, CombineRule::Min),
        (COMBINE_MULTIPLY, CombineRule::Multiply),
        (COMBINE_MAX, CombineRule::Max),
    ] {
        let collider = Collider {
            friction_rule: discriminant,
            restitution_rule: discriminant,
            ..Default::default()
        };
        assert_eq!(collider.material().friction_rule, expected);
        assert_eq!(collider.material().restitution_rule, expected);
    }

    // A scene authored by a newer editor stays loadable rather than
    // failing on a discriminant this build has never heard of.
    let unknown = Collider {
        friction_rule: 99,
        ..Default::default()
    };
    assert_eq!(unknown.material().friction_rule, CombineRule::Average);
}

/// A child collider brings its own surface. An ice patch welded onto a
/// crate is still ice, and the body's material has no business
/// overriding it.
#[test]
fn a_childs_friction_changes_the_bodys_shapes() {
    let mut resources = world();
    let parent = spawn_body(
        &mut resources,
        Transform::default(),
        PhysicsBody::default(),
        Collider::default(),
    );
    let child = spawn_bare(&mut resources);
    insert(&mut resources, child, Transform::default());
    insert(
        &mut resources,
        child,
        Collider {
            friction: 0.1,
            ..Default::default()
        },
    );
    insert(
        &mut resources,
        child,
        kooch_ecs::hierarchy::Parent { entity: parent },
    );
    insert(
        &mut resources,
        parent,
        kooch_ecs::hierarchy::Children {
            entities: vec![child],
        },
    );
    insert(
        &mut resources,
        parent,
        kooch_ecs::hierarchy::GlobalTransform {
            matrix: glam::Mat4::IDENTITY,
        },
    );
    insert(
        &mut resources,
        child,
        kooch_ecs::hierarchy::GlobalTransform {
            matrix: glam::Mat4::IDENTITY,
        },
    );
    physics_sync_system(&mut resources);
    let before = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .spec(slot_of(&resources, parent).expect("no body"));

    if let Some(registry) = resources.get_mut::<ComponentRegistry>()
        && let Some(storage) = registry.get_cpu_mut::<Collider>()
        && let Some(collider) = storage.get_mut(child)
    {
        collider.friction = 1.9;
    }
    physics_sync_system(&mut resources);

    let after = resources
        .get::<PhysicsWorld>()
        .unwrap()
        .spec(slot_of(&resources, parent).expect("no body"));
    assert_ne!(
        before, after,
        "editing a child's friction did not reach the body's digest",
    );
}
