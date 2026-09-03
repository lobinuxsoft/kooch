use super::*;

/// The claim the whole component rests on: letting go brakes, because
/// the goal is zero and the same term chases it.
#[test]
fn no_steering_brakes() {
    let stop = needed(Vec3::ZERO, Vec3::X * 6.0, 90.0, 1.0 / 60.0);
    assert!(stop.x < -1.0, "should push back against the motion: {stop}");
}

/// And it is a cap, not a scale: an uncapped chase is a teleport.
#[test]
fn the_force_is_capped() {
    let big = needed(Vec3::X * 6.0, Vec3::ZERO, 90.0, 1.0 / 60.0);
    assert!((big.length() - 90.0).abs() < 1e-3, "{big}");
}

/// Under the cap it asks for exactly what closes the gap, so a
/// character at speed stops being pushed instead of being clamped.
#[test]
fn a_small_gap_is_exact() {
    let dt = 1.0 / 60.0;
    let small = needed(Vec3::X * 6.0, Vec3::X * 5.99, 90.0, dt);
    assert!((small.x - 0.01 / dt).abs() < 1e-3, "{small}");
}

/// The goal follows the stick at a fixed rate rather than jumping.
#[test]
fn the_goal_is_chased() {
    let mut goals = WalkGoals::default();
    let entity = Entity::new(1, 0);
    let first = goals.chase(entity, Vec3::X * 6.0, 60.0, 1.0 / 60.0);
    assert!(
        (first.x - 1.0).abs() < 1e-4,
        "one step of 60 m/s^2: {first}"
    );
    let second = goals.chase(entity, Vec3::X * 6.0, 60.0, 1.0 / 60.0);
    assert!(second.x > first.x, "it should keep going: {second}");
}

/// And it stops there rather than overshooting on a slow frame.
#[test]
fn a_long_step_lands_on_it() {
    let mut goals = WalkGoals::default();
    let reached = goals.chase(Entity::new(1, 0), Vec3::X * 6.0, 60.0, 1.0);
    assert_eq!(reached, Vec3::X * 6.0);
}

/// A stick pushed into its own corner must not walk faster diagonally.
#[test]
fn a_corner_is_not_faster() {
    let walk = Walk::default();
    let corner = goal(Vec3::new(1.0, 0.0, 1.0), Vec3::Y, &walk);
    assert!((corner.length() - walk.max_speed).abs() < 1e-4, "{corner}");
}

/// Half a stick is half the speed, so a gamepad is not a keyboard.
#[test]
fn a_half_stick_is_half_speed() {
    let walk = Walk::default();
    let half = goal(Vec3::X * 0.5, Vec3::Y, &walk);
    assert!(
        (half.length() - walk.max_speed * 0.5).abs() < 1e-4,
        "{half}"
    );
}

/// Steering into a slope walks along it, not into it.
#[test]
fn a_goal_is_flattened() {
    let walk = Walk::default();
    let uphill = goal(Vec3::new(0.6, 0.8, 0.0), Vec3::Y, &walk);
    assert!(uphill.y.abs() < 1e-5, "{uphill}");
}

/// The bug this exists for: letting go mid-jump must not stop the
/// character in the air. Momentum is what a jump is.
#[test]
fn no_steering_keeps_momentum() {
    let walk = Walk::default();
    let coasting = drift(Vec3::ZERO, Vec3::X * 6.0, Vec3::Y, &walk, 1.0 / 60.0);
    assert_eq!(coasting, Vec3::ZERO);
}

/// It still steers, at its fraction of the walking acceleration.
#[test]
fn the_air_still_steers() {
    let walk = Walk::default();
    let across = drift(Vec3::Z, Vec3::X * 3.0, Vec3::Y, &walk, 1.0 / 60.0);
    let wanted = walk.acceleration * walk.air_control;
    assert!((across.z - wanted).abs() < 1e-3, "{across}");
}

/// And it cannot become thrust: a body already at speed is not pushed
/// past what it arrived with.
#[test]
fn the_air_is_not_thrust() {
    let walk = Walk::default();
    let dt = 1.0 / 60.0;
    let fast = Vec3::X * 20.0;
    let pushed = drift(Vec3::X, fast, Vec3::Y, &walk, dt);
    assert!((fast + pushed * dt).length() <= 20.0 + 1e-3, "{pushed}");
}

/// A jump taken standing still can still be steered, or air control
/// would only work for someone already moving.
#[test]
fn a_standing_jump_can_steer() {
    let walk = Walk::default();
    let pushed = drift(Vec3::X, Vec3::ZERO, Vec3::Y, &walk, 1.0 / 60.0);
    assert!(pushed.x > 0.0, "{pushed}");
}
