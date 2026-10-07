use super::*;
use glam::{Mat4, Quat};

/// A camera at the origin looking down `-Z`.
fn looking_down_z(fov: f32) -> (PerspectiveCamera, GlobalTransform) {
    (
        PerspectiveCamera {
            fov,
            ..PerspectiveCamera::default()
        },
        GlobalTransform {
            matrix: Mat4::IDENTITY,
        },
    )
}

/// 🔴 The invariant the issue is about: twice as far, twice as big in world units, so the apparent
/// size never changes. Without it a grip is sub-pixel at 50 m.
#[test]
fn twice_as_far_is_twice_as_big() {
    let (camera, transform) = looking_down_z(60.0);
    let near = of(&camera, &transform, Vec3::new(0.0, 0.0, -10.0));
    let far = of(&camera, &transform, Vec3::new(0.0, 0.0, -20.0));

    assert!((far / near - 2.0).abs() < 1e-3, "near={near} far={far}");
}

/// 🔴 Measured along the view axis, not as a straight line. A handle off to the side is further from
/// the eye, and sizing by that distance would grow it as the camera turns.
#[test]
fn the_depth_is_along_the_view() {
    let (camera, transform) = looking_down_z(60.0);
    let centre = of(&camera, &transform, Vec3::new(0.0, 0.0, -10.0));
    // Same depth, far off to the side: 14 m from the eye, still 10 m along the view.
    let edge = of(&camera, &transform, Vec3::new(10.0, 0.0, -10.0));

    assert!((edge - centre).abs() < 1e-3, "centre={centre} edge={edge}");
}

/// A wider lens shows more world per pixel, so the same screen size is more world units.
#[test]
fn a_wider_lens_scales_up() {
    let at = Vec3::new(0.0, 0.0, -10.0);
    let (narrow, transform) = looking_down_z(30.0);
    let (wide, _) = looking_down_z(90.0);

    assert!(of(&wide, &transform, at) > of(&narrow, &transform, at) * 2.0);
}

/// Behind the camera, or on it, still answers with a usable size: an affordance straddling the near
/// plane must not collapse to nothing exactly when it is largest on screen.
#[test]
fn a_point_behind_still_sizes() {
    let (camera, transform) = looking_down_z(60.0);
    for at in [
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, 5.0),
        Vec3::new(0.0, 0.0, 1e6),
    ] {
        let scale = of(&camera, &transform, at);
        assert!(scale > 0.0 && scale.is_finite(), "{at} gave {scale}");
    }
}

/// 🔴 `!(fov > 0.0)`: a NaN fov passes a `<=` test and reaches `tan`, and a NaN scale puts the
/// affordance nowhere at all.
#[test]
fn an_unusable_lens_falls_back() {
    let transform = GlobalTransform {
        matrix: Mat4::IDENTITY,
    };
    for fov in [0.0, -60.0, f32::NAN] {
        let (camera, _) = looking_down_z(fov);
        let scale = of(&camera, &transform, Vec3::new(0.0, 0.0, -10.0));
        assert_eq!(scale, 1.0, "fov {fov}");
    }
}

/// A rotated camera measures along its own forward, not along world `-Z`.
#[test]
fn a_turned_camera_measures_its_own_axis() {
    let camera = PerspectiveCamera {
        fov: 60.0,
        ..PerspectiveCamera::default()
    };
    // Turned to look down +X.
    let transform = GlobalTransform {
        matrix: Mat4::from_rotation_translation(
            Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
            Vec3::ZERO,
        ),
    };
    let ahead = of(&camera, &transform, Vec3::new(10.0, 0.0, 0.0));
    let reference = of(
        &camera,
        &GlobalTransform {
            matrix: Mat4::IDENTITY,
        },
        Vec3::new(0.0, 0.0, -10.0),
    );

    assert!((ahead - reference).abs() < 1e-3, "{ahead} vs {reference}");
}
