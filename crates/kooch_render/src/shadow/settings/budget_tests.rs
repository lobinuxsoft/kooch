use super::*;

/// The uniform declares a fixed array; a budget past it would index off
/// the end of it, which is undefined rather than merely wrong.
#[test]
fn the_budget_cannot_exceed_the_uniform() {
    let settings = ShadowSettings {
        point_shadows: 9_000,
        ..Default::default()
    };
    assert_eq!(settings.point_budget(), kooch_lighting::MAX_POINT_SHADOWS,);
}

/// Zero is a real answer — a project that wants no cube maps should not
/// allocate the array — and must not be turned into the default.
#[test]
fn zero_cubes_is_expressible() {
    let settings = ShadowSettings {
        point_shadows: 0,
        ..Default::default()
    };
    assert_eq!(settings.point_budget(), 0);
}

/// The default is what every capture on record was taken against.
#[test]
fn the_default_is_still_four() {
    assert_eq!(ShadowSettings::default().point_budget(), 4);
}
