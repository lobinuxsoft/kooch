use super::*;

/// The reason there are two deadzones and not one parameterised.
///
/// A per-axis deadzone leaves a square hole: a stick pushed diagonally
/// clears it while the same magnitude along an axis does not. This is
/// the bug #57 names, drawn as a test.
#[test]
fn only_the_stick_deadzone_leaves_a_round_hole() {
    let (min, max) = (0.2, 0.9);
    // Just inside the deadzone, straight up.
    let cardinal = Vec2::new(0.0, 0.15);
    // The same length, at 45°.
    let diagonal = Vec2::new(0.15, 0.15).normalize() * 0.15;
    assert!((cardinal.length() - diagonal.length()).abs() < 1e-5);

    let radial = Processor::StickDeadzone { min, max };
    assert_eq!(radial.apply_vec2(cardinal), Vec2::ZERO);
    assert_eq!(
        radial.apply_vec2(diagonal),
        Vec2::ZERO,
        "a round deadzone must reject both, since they are the same push"
    );

    // Per-axis: each component alone is under `min`, so the diagonal
    // is rejected too — but push it a little further and the corner
    // survives while the cardinal of equal length does not.
    let square = Processor::AxisDeadzone { min, max };
    let long_diagonal = Vec2::new(0.25, 0.25);
    let long_cardinal = Vec2::new(0.0, long_diagonal.length());
    assert_ne!(square.apply_vec2(long_diagonal), Vec2::ZERO);
    assert_ne!(
        square.apply_vec2(long_cardinal),
        Vec2::ZERO,
        "sanity: both are past min"
    );
    // …and the shapes differ, which is the whole point.
    assert!(
        (square.apply_vec2(long_diagonal).length() - square.apply_vec2(long_cardinal).length())
            .abs()
            > 0.1,
        "a square deadzone should distort direction; if this fails the \
             two processors have collapsed into one"
    );
}

/// Leaving the deadzone must start at zero, not jump to `min`.
#[test]
fn a_value_leaving_the_deadzone_starts_from_zero() {
    let p = Processor::AxisDeadzone { min: 0.2, max: 0.9 };
    assert_eq!(p.apply(0.19), 0.0);
    assert!(p.apply(0.2001).abs() < 0.01, "stepped instead of easing in");
    assert_eq!(p.apply(0.95), 1.0, "past max is already full");
    assert_eq!(p.apply(-0.95), -1.0, "and symmetric");
}

/// A nudge stays a nudge; a diagonal does not outrun a straight line.
/// This is `clamp_to_unit` from the game, promoted to a processor.
#[test]
fn normalising_a_vector_caps_without_stretching() {
    let p = Processor::NormalizeVector2;
    let half = Vec2::new(0.0, 0.5);
    assert_eq!(p.apply_vec2(half), half, "a half-held stick became full");

    let diagonal = p.apply_vec2(Vec2::new(1.0, 1.0));
    assert!(
        (diagonal.length() - 1.0).abs() < 1e-5,
        "diagonal travels {}× too fast",
        diagonal.length()
    );
}

/// Order matters, and the list is the order — no hidden stage before
/// or after, which is the thing three processor sites cost you.
#[test]
fn processors_apply_in_the_order_they_are_listed() {
    let value = -0.5;
    let invert_then_scale = Processor::Scale { factor: 2.0 }.apply(Processor::Invert.apply(value));
    let scale_then_invert = Processor::Invert.apply(Processor::Scale { factor: 2.0 }.apply(value));
    assert_eq!(invert_then_scale, 1.0);
    assert_eq!(scale_then_invert, 1.0);

    // Deadzone is where order shows: scaling first can push a value
    // out of a deadzone that would have swallowed it.
    let dead = Processor::AxisDeadzone { min: 0.2, max: 0.9 };
    let small = 0.15;
    assert_eq!(dead.apply(small), 0.0);
    assert_ne!(
        dead.apply(Processor::Scale { factor: 2.0 }.apply(small)),
        0.0
    );
}
