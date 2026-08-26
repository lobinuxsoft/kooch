use super::*;

fn close(actual: Vec3, expected: Vec3) -> bool {
    actual.abs_diff_eq(expected, 1e-3)
}

/// Rapier's palette is HSLA with hue in degrees, not the 0..1 most
/// colour code assumes. Reading it as normalised would turn every
/// collider the same shade.
#[test]
fn hue_is_read_in_degrees() {
    assert!(close(
        hsla_to_rgb([0.0, 1.0, 0.5, 1.0]),
        Vec3::new(1.0, 0.0, 0.0)
    ));
    assert!(close(
        hsla_to_rgb([120.0, 1.0, 0.5, 1.0]),
        Vec3::new(0.0, 1.0, 0.0)
    ));
    assert!(close(
        hsla_to_rgb([240.0, 1.0, 0.5, 1.0]),
        Vec3::new(0.0, 0.0, 1.0)
    ));
}

#[test]
fn zero_saturation_is_grey() {
    let grey = hsla_to_rgb([200.0, 0.0, 0.5, 1.0]);
    assert!(close(grey, Vec3::splat(0.5)), "{grey}");
}

/// Rapier darkens a sleeping body by scaling its lightness. If that
/// did not survive the conversion, "why did this stop reacting" stays
/// unanswerable.
#[test]
fn a_darker_lightness_gives_a_darker_colour() {
    let awake = hsla_to_rgb([340.0, 1.0, 0.3, 1.0]);
    let asleep = hsla_to_rgb([340.0, 1.0, 0.3 * 0.2, 1.0]);
    assert!(
        asleep.length() < awake.length(),
        "asleep {asleep} is not darker than awake {awake}",
    );
}

/// A hue of exactly 360 is the same colour as 0, and an out-of-range
/// one must not index past the sector table.
#[test]
fn hue_wraps_instead_of_falling_off_the_end() {
    assert!(close(
        hsla_to_rgb([360.0, 1.0, 0.5, 1.0]),
        hsla_to_rgb([0.0, 1.0, 0.5, 1.0]),
    ));
    assert!(hsla_to_rgb([720.0, 1.0, 0.5, 1.0]).is_finite());
    assert!(hsla_to_rgb([-40.0, 1.0, 0.5, 1.0]).is_finite());
}

/// Every switch has to reach a rapier flag, or ticking a box in the
/// editor draws nothing and looks like a broken overlay.
#[test]
fn each_category_maps_to_a_rapier_flag() {
    assert!(mode_for(DebugCategories::default()).is_empty());
    let cases = [
        DebugCategories {
            collider_shapes: true,
            ..Default::default()
        },
        DebugCategories {
            contacts: true,
            ..Default::default()
        },
        DebugCategories {
            joints: true,
            ..Default::default()
        },
        DebugCategories {
            collider_aabbs: true,
            ..Default::default()
        },
        DebugCategories {
            body_axes: true,
            ..Default::default()
        },
    ];
    for case in cases {
        assert!(
            !mode_for(case).is_empty(),
            "{case:?} maps to no rapier flag",
        );
    }
    assert_eq!(mode_for(DebugCategories::all()), DebugRenderMode::all());
}
