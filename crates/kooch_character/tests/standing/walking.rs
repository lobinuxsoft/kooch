//! Walking: steps, facing, jumps, stopping, top speed, momentum, ramps and slides.

use super::*;

/// Walks the character at `entity` along +X for `steps` frames, at a
/// walking pace, and reports the highest it ever got.
fn walk(resources: &mut Resources, entity: Entity, steps: u32) -> f32 {
    let mut highest = f32::MIN;
    for _ in 0..steps {
        let body = resources
            .get::<ComponentRegistry>()
            .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
            .and_then(|s| s.get(entity))
            .copied();
        if let Some(body) = body
            && let Some(world) = resources.get_mut::<PhysicsWorld>()
        {
            let speed = world.linear_velocity(body).unwrap_or(Vec3::ZERO).x;
            if speed < 3.0 {
                world.apply_impulse(body, Vec3::X * 0.05);
            }
        }
        simulate(resources, 1);
        highest = highest.max(position(resources, entity).y);
    }
    highest
}

/// How far up a riser of `height` the character gets, in metres.
fn onto_a_step(height: f32) -> f32 {
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
        Vec3::new(4.0, 0.5, 6.0),
    );
    // Long enough that reaching the top is the only way past it.
    floor(
        &mut resources,
        Vec3::new(10.0, height - 2.5, 0.0),
        Vec3::new(6.0, 2.0, 6.0),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 0.5, 0.0));
    simulate(&mut resources, 180);
    let start = position(&resources, hero).y;
    walk(&mut resources, hero, 300) - start
}

/// The headline claim: a step is climbed by the spring alone, with
/// nothing in the code that knows what a step is.
#[test]
fn a_low_step_is_climbed() {
    let rose = onto_a_step(0.6);
    assert!(rose > 0.55, "should have got up a 0.6 m step, rose {rose}");
}

/// And the other half — the same mechanism has to refuse a wall, or
/// "climbs steps" means "walks through the level".
#[test]
fn a_tall_step_is_not() {
    let rose = onto_a_step(1.0);
    assert!(
        rose < 0.2,
        "should have been stopped by a 1 m wall, rose {rose}"
    );
}

/// Acceptance: it points where it is steered. Without a `Facing` the controller only ever stood the
/// body up, and a character that walks sideways for ever is what that looks like.
#[test]
fn it_faces_where_it_walks() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);

    let steered = Vec3::new(1.0, 0.0, 1.0).normalize();
    insert(&mut resources, hero, Facing { direction: steered });
    simulate(&mut resources, 120);

    let looking = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(hero))
        .map(|t| t.rotation * Vec3::NEG_Z)
        .expect("no transform");
    assert!(
        looking.dot(steered) > 0.99,
        "should look along {steered}, looked along {looking}",
    );
}

/// Acceptance: a jump leaves the floor, which needs the spring to let go while rising.
#[test]
fn a_jump_leaves_the_ground() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    let resting = position(&resources, hero).y;

    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        let mass = world.mass(body).unwrap_or(1.0);
        world.apply_impulse(body, Vec3::Y * 5.0 * mass);
    }

    let mut highest = resting;
    for _ in 0..120 {
        simulate(&mut resources, 1);
        highest = highest.max(position(&resources, hero).y);
    }
    // 5 m/s against 9.81 is 1.27 m of arc. Anything under half of that is the spring winning.
    assert!(
        highest - resting > 0.6,
        "jumped {} m from {resting}",
        highest - resting,
    );
}

/// A landing that dips and comes back. At critical damping the body arrives dead — bottomed and
/// settled agree to seven decimals — which is correct and reads as a character with no weight.
#[test]
fn it_dips_when_it_lands() {
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
    let hero = character(&mut resources, Vec3::new(0.0, 5.0, 0.0));

    let mut lowest = f32::MAX;
    for _ in 0..240 {
        simulate(&mut resources, 1);
        lowest = lowest.min(position(&resources, hero).y);
    }
    let settled = position(&resources, hero).y;
    assert!(
        settled - lowest > 0.08,
        "should have dipped and recovered: bottomed at {lowest}, settled at {settled}",
    );
}

/// Acceptance: it stops when you let go — a frictionless capsule needs stopping to be the same term
/// as starting.
#[test]
fn it_stops_when_you_let_go() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());

    let cruising = walked(&mut resources, hero, Vec3::X, 180);
    assert!(cruising > 4.0, "should be walking: {cruising}");

    let stopped = walked(&mut resources, hero, Vec3::ZERO, 60);
    assert!(stopped < 0.3, "should have stopped: {stopped} m/s");
}

/// And the top speed is the goal's, not a clamp applied afterwards.
#[test]
fn it_holds_its_top_speed() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    let steps = Walk::default();
    insert(&mut resources, hero, steps);

    let reached = walked(&mut resources, hero, Vec3::X, 240);
    assert!(
        (reached - steps.max_speed).abs() < 0.4,
        "wanted {}, reached {reached}",
        steps.max_speed,
    );
}

/// Nobody steering is nobody moving. The spring and the lean both act along the local up, and a
/// stationary character that drifts means one of them is leaking sideways.
#[test]
fn a_standing_character_does_not_drift() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());

    insert(
        &mut resources,
        hero,
        Facing {
            direction: Vec3::ZERO,
        },
    );
    let start = position(&resources, hero);
    simulate(&mut resources, 240);
    let drift = position(&resources, hero) - start;
    let across = Vec3::new(drift.x, 0.0, drift.z).length();
    assert!(across < 0.1, "it drifted {across} m");
}

/// Acceptance: letting go mid-jump keeps the momentum, where the ground chase would stop it dead.
#[test]
fn a_jump_keeps_its_momentum() {
    let mut resources = world();
    let hero = on_the_floor(&mut resources);
    insert(&mut resources, hero, Walk::default());

    let running = walked(&mut resources, hero, Vec3::X, 180);
    assert!(running > 4.0, "should be walking: {running}");

    let body = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<kooch_physics::plugin::SolverBody>())
        .and_then(|s| s.get(hero))
        .copied()
        .expect("no body");
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        let mass = world.mass(body).unwrap_or(1.0);
        world.apply_impulse(body, Vec3::Y * 6.0 * mass);
    }

    // Off the ground, and the stick released at the top of the arc.
    simulate(&mut resources, 20);
    let coasting = walked(&mut resources, hero, Vec3::ZERO, 25);
    assert!(
        coasting > running * 0.8,
        "it stopped in mid-air: {running} became {coasting}",
    );
}

/// It stays upright against the field, on a ramp as anywhere else.
#[test]
fn a_ramp_does_not_tip_it() {
    let mut resources = world();
    Playing::set(&mut resources, true);
    source_at(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        GlobalGravity::default(),
    );
    let tilt = 25f32.to_radians();
    slab(
        &mut resources,
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(12.0, 0.5, 12.0),
        glam::Quat::from_rotation_z(tilt),
    );
    let hero = character(&mut resources, Vec3::new(0.0, 2.0, 0.0));
    simulate(&mut resources, 300);

    let standing = resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(hero))
        .map(|t| t.rotation * Vec3::Y)
        .expect("no transform");
    assert!(
        standing.y > 0.999,
        "should stand straight up on a ramp: {standing}",
    );
}

/// Acceptance: walking up a ramp is not leaving the ground — measured along the field, 6 m/s up 25°
/// read as a jump.
#[test]
fn a_climb_is_still_standing() {
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
        Vec3::new(16.0, 0.5, 12.0),
        glam::Quat::from_rotation_z(25f32.to_radians()),
    );
    let hero = character(&mut resources, Vec3::new(-4.0, 4.0, 0.0));
    insert(&mut resources, hero, Walk::default());
    simulate(&mut resources, 240);

    // Uphill, at walking pace, for long enough that a frame is not luck.
    let mut refused = 0;
    insert(&mut resources, hero, Facing { direction: Vec3::X });
    for _ in 0..180 {
        simulate(&mut resources, 1);
        if !grounded(&resources, hero).standing {
            refused += 1;
        }
    }
    assert!(refused < 10, "lost the ground {refused} frames out of 180");
}

/// Acceptance: a slope too steep to walk takes the character back down; a riser's matching normal
/// is why a ledge decides it.
#[test]
fn a_steep_slope_slides() {
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
        Vec3::new(16.0, 0.5, 12.0),
        glam::Quat::from_rotation_z(65f32.to_radians()),
    );
    let hero = character(&mut resources, Vec3::new(-2.0, 6.0, 0.0));
    insert(&mut resources, hero, Walk::default());
    simulate(&mut resources, 60);

    // Walking straight up it, as hard as it can.
    let start = position(&resources, hero).y;
    insert(&mut resources, hero, Facing { direction: Vec3::X });
    simulate(&mut resources, 180);
    let ended = position(&resources, hero).y;

    assert!(
        ended < start,
        "should have slid down a 65 degree slope, went from {start} to {ended}",
    );
    assert!(
        !grounded(&resources, hero).standing,
        "and never counted as standing on it",
    );
}
