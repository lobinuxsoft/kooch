/// A loop with nothing to draw paces itself off this. Zero means a
/// step is already owed, so the caller must not treat it as "sleep
/// forever" (#656).
#[test]
fn the_wait_for_the_next_fixed_step_shrinks_as_time_accumulates() {
    let mut time = Time::new();
    time.set_fixed_hz(60.0);
    let step = time.fixed_delta();

    // Fresh: a whole step away.
    assert_eq!(time.until_next_fixed_step(), step);

    // Two thirds of a step in, so a third of a step to go.
    time.advance(step.mul_f32(2.0 / 3.0));
    let remaining = time.until_next_fixed_step();
    assert!(
        remaining < step && remaining > Duration::ZERO,
        "expected part of a step, got {remaining:?}",
    );

    // Advancing exactly one step runs one and leaves the *phase*
    // intact — the accumulator keeps the remainder rather than being
    // drained. A pacer that assumed a reset here would sleep a third
    // of a step too long, every frame, and the sim would run slow.
    assert_eq!(time.advance(step), 1);
    assert_eq!(
        time.until_next_fixed_step(),
        remaining,
        "the remainder was dropped instead of carried",
    );
}

use super::*;

#[test]
fn default_fixed_rate() {
    let time = Time::new();
    let hz = time.fixed_hz();
    assert!((hz - 60.0).abs() < 0.1);
}

#[test]
fn set_fixed_hz() {
    let mut time = Time::new();
    time.set_fixed_hz(120.0);
    let hz = time.fixed_hz();
    assert!((hz - 120.0).abs() < 0.1);
}

#[test]
fn advance_calculates_fixed_updates() {
    let mut time = Time::new();

    // Advance by exactly one fixed timestep
    let updates = time.advance(time.fixed_delta());
    assert_eq!(updates, 1);
    assert_eq!(time.fixed_count(), 1);

    // Advance by 100ms (should be 5-6 updates at 60Hz depending on float precision)
    // 100ms / 16.67ms ≈ 6, but floating point may give 5
    let updates = time.advance(Duration::from_millis(100));
    assert!(
        updates >= 5 && updates <= 6,
        "Expected 5-6 updates, got {}",
        updates
    );
}

#[test]
fn render_alpha_calculation() {
    let mut time = Time::new();

    // Advance by half a fixed timestep
    let half_step = Duration::from_secs_f64(1.0 / 120.0);
    time.advance(half_step);

    // render_alpha should be ~0.5
    assert!((time.render_alpha() - 0.5).abs() < 0.01);
}

#[test]
fn max_accumulator_prevents_spiral_of_death() {
    let mut time = Time::new();

    // Advance by a huge amount (simulating a freeze)
    let updates = time.advance(Duration::from_secs(1));

    // Should be capped at max_accumulator / fixed_delta = 250ms / 16.67ms ≈ 15
    assert!(updates <= 15);
}

#[test]
fn elapsed_accumulates() {
    let mut time = Time::new();

    time.advance(Duration::from_millis(16));
    time.advance(Duration::from_millis(16));
    time.advance(Duration::from_millis(16));

    assert_eq!(time.elapsed(), Duration::from_millis(48));
    assert_eq!(time.frame_count(), 3);
}
