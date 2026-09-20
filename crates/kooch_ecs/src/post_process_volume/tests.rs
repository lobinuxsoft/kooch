use super::*;

fn volume(blend_distance: f32, weight: f32) -> PostProcessVolume {
    PostProcessVolume {
        blend_distance,
        weight,
        ..Default::default()
    }
}

#[test]
fn the_surface_is_none_of_it() {
    assert_eq!(volume(4.0, 1.0).weight_at(0.0), 0.0);
}

#[test]
fn a_blend_distance_in_is_all_of_it() {
    assert_eq!(volume(4.0, 1.0).weight_at(4.0), 1.0);
    assert_eq!(volume(4.0, 1.0).weight_at(40.0), 1.0);
}

/// 🔴 The volume's own weight is a ceiling, not a second ramp: half of a volume that has arrived is
/// half the effect, everywhere inside it.
#[test]
fn the_weight_is_a_ceiling() {
    assert_eq!(volume(4.0, 0.5).weight_at(4.0), 0.5);
}

/// Zero blend is a cut — and not a division by zero.
#[test]
fn a_zero_blend_cuts() {
    assert_eq!(volume(0.0, 1.0).weight_at(0.0), 1.0);
}

/// Outside is outside: a depth the sensor reports as negative contributes nothing.
#[test]
fn outside_contributes_nothing() {
    assert_eq!(volume(4.0, 1.0).weight_at(-0.1), 0.0);
}

#[test]
fn a_disabled_volume_is_off() {
    let off = PostProcessVolume {
        enabled: false,
        ..volume(4.0, 1.0)
    };
    assert_eq!(off.weight_at(f32::INFINITY), 0.0);
}
