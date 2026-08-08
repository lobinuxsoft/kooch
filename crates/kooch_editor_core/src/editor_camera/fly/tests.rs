use super::*;

fn approx_eq(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a - b).length() < eps
}

#[test]
fn no_keys_returns_zero() {
    let v = fly_velocity(FlyKeys::default(), Quat::IDENTITY, 5.0, 0.016);
    assert_eq!(v, Vec3::ZERO);
}

#[test]
fn forward_moves_along_minus_z_for_identity_orientation() {
    let keys = FlyKeys {
        forward: true,
        ..Default::default()
    };
    let v = fly_velocity(keys, Quat::IDENTITY, 10.0, 1.0);
    assert!(approx_eq(v, Vec3::new(0.0, 0.0, -10.0), 1e-5));
}

#[test]
fn opposing_keys_cancel() {
    let keys = FlyKeys {
        forward: true,
        backward: true,
        left: true,
        right: true,
        ..Default::default()
    };
    let v = fly_velocity(keys, Quat::IDENTITY, 10.0, 1.0);
    assert_eq!(v, Vec3::ZERO);
}

#[test]
fn diagonal_movement_is_normalised() {
    // Forward + right at speed 10 / dt 1 should still have length 10,
    // not 10 * sqrt(2).
    let keys = FlyKeys {
        forward: true,
        right: true,
        ..Default::default()
    };
    let v = fly_velocity(keys, Quat::IDENTITY, 10.0, 1.0);
    assert!((v.length() - 10.0).abs() < 1e-5);
}

#[test]
fn vertical_keys_use_world_up_not_camera_up() {
    // Tilt camera 60° pitch down; pressing E should still go +Y world.
    let pitched = Quat::from_axis_angle(Vec3::X, std::f32::consts::FRAC_PI_3);
    let keys = FlyKeys {
        up: true,
        ..Default::default()
    };
    let v = fly_velocity(keys, pitched, 5.0, 1.0);
    assert!(approx_eq(v, Vec3::new(0.0, 5.0, 0.0), 1e-5));
}

#[test]
fn zero_dt_yields_zero() {
    let keys = FlyKeys {
        forward: true,
        ..Default::default()
    };
    let v = fly_velocity(keys, Quat::IDENTITY, 5.0, 0.0);
    assert_eq!(v, Vec3::ZERO);
}

#[test]
fn zero_speed_yields_zero() {
    let keys = FlyKeys {
        forward: true,
        ..Default::default()
    };
    let v = fly_velocity(keys, Quat::IDENTITY, 0.0, 0.016);
    assert_eq!(v, Vec3::ZERO);
}

#[test]
fn any_returns_correct_state() {
    assert!(!FlyKeys::default().any());
    assert!(
        FlyKeys {
            forward: true,
            ..Default::default()
        }
        .any()
    );
    assert!(
        FlyKeys {
            down: true,
            ..Default::default()
        }
        .any()
    );
}
