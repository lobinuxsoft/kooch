use super::*;

/// Rapier's defaults are the sane ones; #623 exposed them rather than
/// changing them, and a silent change here would alter every existing
/// scene.
#[test]
fn the_defaults_are_rapiers_own() {
    let material = SurfaceMaterial::default();
    assert_eq!(material.friction, 0.5);
    assert_eq!(material.restitution, 0.0);
    assert_eq!(material.friction_rule, CombineRule::Average);
    assert_eq!(material.restitution_rule, CombineRule::Average);
    assert_eq!(Damping::default(), Damping::default().sanitised());
}

/// A negative coefficient is not a slipperier surface — it is a solver
/// that pushes bodies together when they separate.
#[test]
fn negative_coefficients_are_clamped_away() {
    let material = SurfaceMaterial {
        friction: -1.0,
        restitution: -0.5,
        ..Default::default()
    }
    .sanitised();
    assert_eq!(material.friction, 0.0);
    assert_eq!(material.restitution, 0.0);
}

/// A negative damping adds energy every step, and a body that gains
/// energy forever leaves the number line.
#[test]
fn negative_damping_is_clamped_away() {
    let damping = Damping {
        linear: -2.0,
        angular: -3.0,
    }
    .sanitised();
    assert_eq!((damping.linear, damping.angular), (0.0, 0.0));
}

/// Clamping must not quietly rewrite the rules while fixing the
/// numbers.
#[test]
fn sanitising_keeps_the_combine_rules() {
    let material = SurfaceMaterial {
        friction: -1.0,
        friction_rule: CombineRule::Multiply,
        restitution_rule: CombineRule::Max,
        ..Default::default()
    }
    .sanitised();
    assert_eq!(material.friction_rule, CombineRule::Multiply);
    assert_eq!(material.restitution_rule, CombineRule::Max);
}
