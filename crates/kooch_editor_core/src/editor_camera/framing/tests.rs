use glam::Vec3;

use super::{distance_for, radius_around};

/// The editor's default vertical field of view.
const FOV: f32 = std::f32::consts::FRAC_PI_3;

#[test]
fn nothing_to_frame_stands_five_units_off() {
    // An entity with a transform and no mesh: a spawn point, a trigger.
    assert_eq!(distance_for(None, FOV), 5.0);
}

#[test]
fn a_zero_radius_is_nothing_to_frame() {
    // A degenerate bound is not "put the camera inside it".
    assert_eq!(distance_for(Some(0.0), FOV), 5.0);
}

#[test]
fn a_unit_sphere_fits_at_two_units() {
    // 60° vertical fov: sin(30°) = 0.5, so d = r / 0.5 = 2r.
    assert!((distance_for(Some(1.0), FOV) - 2.0).abs() < 1e-4);
}

#[test]
fn a_bigger_thing_is_framed_from_further() {
    let near = distance_for(Some(1.0), FOV);
    let far = distance_for(Some(4.0), FOV);
    assert!((far - near * 4.0).abs() < 1e-3, "{far} vs {near}");
}

#[test]
fn a_wider_lens_frames_from_closer() {
    let narrow = distance_for(Some(1.0), 0.4);
    let wide = distance_for(Some(1.0), 1.2);
    assert!(wide < narrow);
}

#[test]
fn a_speck_is_not_framed_inside_the_near_plane() {
    // A single selected vertex has no size at all.
    assert!(distance_for(Some(0.0001), FOV) >= 0.35);
}

#[test]
fn the_radius_reaches_the_furthest_corner() {
    // A unit cube centred on the origin: half-diagonal is √3 / 2.
    let radius = radius_around(Vec3::ZERO, Vec3::splat(-0.5), Vec3::splat(0.5));
    assert!((radius - 0.8660254).abs() < 1e-5, "{radius}");
}

#[test]
fn an_off_centre_aim_reaches_the_far_side() {
    // 🔴 Measured from where the camera AIMS, not from the box's middle.
    // A face selection's centre sits on the box's edge, and half the
    // diagonal would frame a point nobody is looking at.
    let radius = radius_around(
        Vec3::new(0.5, 0.0, 0.0),
        Vec3::splat(-0.5),
        Vec3::splat(0.5),
    );
    assert!(radius > 0.8660254, "{radius}");
}
