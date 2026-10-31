/// Advancing one whole step runs one and keeps the *phase*: the accumulator carries the remainder
/// rather than being drained, or interpolation would snap back every frame.
#[test]
fn a_step_keeps_the_remainder() {
    let mut time = Time::new();
    time.set_fixed_hz(60.0);
    let step = time.fixed_delta();

    time.advance(step.mul_f32(2.0 / 3.0));
    let phase = time.render_alpha();
    assert!((phase - 2.0 / 3.0).abs() < 1e-3, "{phase}");

    assert_eq!(time.advance(step), 1);
    assert!(
        (time.render_alpha() - phase).abs() < 1e-3,
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

/// Paused gameplay runs no fixed step, while the UI's real clock still moves.
#[test]
fn a_zero_scale_pauses() {
    let mut time = Time::new();
    time.set_scale(0.0);
    assert_eq!(time.advance(Duration::from_millis(100)), 0);
    assert_eq!(time.delta(), Duration::ZERO);
    assert_eq!(time.unscaled_delta(), Duration::from_millis(100));
}
