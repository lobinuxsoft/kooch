/// The property the whole migration is for: with no far plane,
/// `ndc.z` **is** `near / distance`, so a shader recovers metres
/// with one divide and no uniform. Checked against the matrix
/// rather than against the algebra that motivated it.
#[test]
fn infinite_reverse_z_makes_ndc_z_exactly_near_over_distance() {
    let near = 0.1;
    let proj = super::perspective_infinite_rh_reverse_z(60.0_f32.to_radians(), 16.0 / 9.0, near);
    for distance in [0.1_f32, 1.0, 37.0, 1_000.0, 100_000.0] {
        let clip = proj * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
        let ndc_z = clip.z / clip.w;
        let expected = near / distance;
        assert!(
            (ndc_z - expected).abs() < expected * 1e-5,
            "at {distance} m ndc.z was {ndc_z}, not {expected}",
        );
    }
}

/// Near maps to 1 and there is no distance that reaches 0 — which
/// is the reversed-Z half of the contract, and what the depth
/// clear and the `Greater` comparison depend on.
#[test]
fn near_is_one_and_infinity_only_approaches_zero() {
    let near = 0.1;
    let proj = super::perspective_infinite_rh_reverse_z(60.0_f32.to_radians(), 1.0, near);
    let ndc = |d: f32| {
        let clip = proj * glam::Vec4::new(0.0, 0.0, -d, 1.0);
        clip.z / clip.w
    };
    assert!((ndc(near) - 1.0).abs() < 1e-6, "near was {}", ndc(near));
    assert!(ndc(1.0e9) > 0.0, "something reached the far plane");
    assert!(ndc(1.0e9) < 1.0e-6);
}

/// The shader reads `near` back out of the matrix. If this element
/// ever stops being it, every depth linearisation downstream is
/// scaled by a number nobody chose.
#[test]
fn near_is_recoverable_from_the_matrix() {
    let near = 0.37;
    let proj = super::perspective_infinite_rh_reverse_z(75.0_f32.to_radians(), 1.5, near);
    assert!((super::near_from_infinite_projection(proj) - near).abs() < 1e-6);
}

use super::*;
use glam::{Vec3, Vec4Swizzles};

fn project_point(proj: Mat4, view_z: f32) -> f32 {
    let clip = proj * Vec4::new(0.0, 0.0, view_z, 1.0);
    clip.z / clip.w
}

#[test]
fn near_plane_maps_to_one() {
    let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    // RH cameras look down -Z; the near plane is at view_z = -near.
    let z = project_point(proj, -0.1);
    assert!(
        (z - 1.0).abs() < 1e-3,
        "near plane should map to ndc.z ≈ 1.0, got {z}"
    );
}

#[test]
fn far_plane_maps_to_zero() {
    let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let z = project_point(proj, -100.0);
    assert!(
        z.abs() < 1e-3,
        "far plane should map to ndc.z ≈ 0.0, got {z}"
    );
}

#[test]
fn midpoint_lies_between() {
    // Reversed-Z spreads precision NON-uniformly in view space —
    // points closer to the camera get more depth resolution. The
    // mid-distance point lands somewhere between 0 and 1, NOT
    // exactly 0.5, but ordering is preserved monotonically with
    // distance.
    let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let z_close = project_point(proj, -1.0);
    let z_mid = project_point(proj, -50.0);
    let z_far = project_point(proj, -100.0);
    assert!(
        z_close > z_mid,
        "closer point must have larger ndc.z (reversed-Z)"
    );
    assert!(z_mid > z_far, "mid point must have larger ndc.z than far");
    assert!((0.0..=1.0).contains(&z_mid), "ndc.z must stay in [0, 1]");
}

#[test]
fn xy_unchanged_versus_standard() {
    // The depth flip only touches z; xy must match standard perspective.
    let std = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let rev = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let p = Vec4::new(1.0, 0.5, -10.0, 1.0);
    let s = std * p;
    let r = rev * p;
    assert_eq!(s.x, r.x);
    assert_eq!(s.y, r.y);
    assert_eq!(s.w, r.w);
}

#[test]
fn world_corner_round_trip() {
    // Sanity: a world-space point at the centre of the frustum
    // projects somewhere visible in NDC.
    let proj = perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
    let view_proj = proj * view;
    let p = view_proj * Vec4::new(0.0, 0.0, 0.0, 1.0);
    let ndc = p.xyz() / p.w;
    assert!(ndc.x.abs() < 0.5);
    assert!(ndc.y.abs() < 0.5);
    // Origin is between near (5 - 0.1) and far (5 + 100), should
    // give a smallish ndc.z (closer to 0 than to 1 — well into the
    // reversed-Z far band).
    assert!((0.0..=1.0).contains(&ndc.z));
}
/// The centre pixel points straight down the camera's forward axis.
/// Camera at +5Z looking at the origin means forward is -Z.
#[test]
fn the_centre_of_the_viewport_looks_where_the_camera_looks() {
    let camera = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y).inverse();
    let ray = viewport_cursor_to_ray(
        Vec2::new(400.0, 300.0),
        Vec2::new(800.0, 600.0),
        camera,
        60.0_f32.to_radians(),
        0.1,
    )
    .expect("a centred cursor in a real viewport has a ray");
    assert!((ray.origin - Vec3::new(0.0, 0.0, 5.0)).length() < 1e-4);
    assert!(
        (ray.direction - Vec3::NEG_Z).length() < 1e-3,
        "expected forward, got {:?}",
        ray.direction,
    );
}

/// Reversed-Z is the trap: unproject with the conventional orientation
/// and the ray comes out pointing *behind* the camera. The dot product
/// against forward catches exactly that, which a length check would not.
#[test]
fn the_ray_leaves_the_camera_rather_than_entering_it() {
    let camera = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y).inverse();
    for cursor in [
        Vec2::new(10.0, 10.0),
        Vec2::new(790.0, 10.0),
        Vec2::new(400.0, 590.0),
    ] {
        let ray = viewport_cursor_to_ray(
            cursor,
            Vec2::new(800.0, 600.0),
            camera,
            60.0_f32.to_radians(),
            0.1,
        )
        .unwrap();
        assert!(
            ray.direction.dot(Vec3::NEG_Z) > 0.0,
            "cursor {cursor:?} produced a ray pointing backwards: {:?}",
            ray.direction,
        );
    }
}

/// egui's Y grows downward. A cursor above centre must map to a ray
/// aiming upward in world space, and a flipped sign here is invisible
/// in a symmetric test.
#[test]
fn screen_y_is_flipped_into_world_y() {
    let camera = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y).inverse();
    let size = Vec2::new(800.0, 600.0);
    let fov = 60.0_f32.to_radians();
    let above = viewport_cursor_to_ray(Vec2::new(400.0, 100.0), size, camera, fov, 0.1).unwrap();
    let below = viewport_cursor_to_ray(Vec2::new(400.0, 500.0), size, camera, fov, 0.1).unwrap();
    assert!(above.direction.y > 0.0, "top of screen should aim up");
    assert!(below.direction.y < 0.0, "bottom of screen should aim down");
}

#[test]
fn a_viewport_with_no_area_has_no_ray() {
    let camera = Mat4::IDENTITY;
    for size in [Vec2::ZERO, Vec2::new(800.0, 0.0), Vec2::new(0.0, 600.0)] {
        assert!(
            viewport_cursor_to_ray(Vec2::ZERO, size, camera, 1.0, 0.1).is_none(),
            "size {size:?} should not produce a ray",
        );
    }
}

#[test]
fn a_ray_aimed_down_meets_the_ground() {
    let ray = WorldRay {
        origin: Vec3::new(2.0, 10.0, -3.0),
        direction: Vec3::new(0.0, -1.0, 0.0),
    };
    let hit = ray.hits_horizontal_plane(0.0).unwrap();
    assert!((hit - Vec3::new(2.0, 0.0, -3.0)).length() < 1e-5);
}

/// Both the parallel case and the behind-the-camera case. Placing
/// something at the algebraic solution of either would put it somewhere
/// the user did not click — off at the horizon, or behind them.
#[test]
fn a_ray_that_never_reaches_the_ground_ahead_reports_nothing() {
    let parallel = WorldRay {
        origin: Vec3::new(0.0, 5.0, 0.0),
        direction: Vec3::X,
    };
    assert_eq!(parallel.hits_horizontal_plane(0.0), None);

    let upward = WorldRay {
        origin: Vec3::new(0.0, 5.0, 0.0),
        direction: Vec3::Y,
    };
    assert_eq!(
        upward.hits_horizontal_plane(0.0),
        None,
        "the plane is behind the camera, not in front of it",
    );
}

#[test]
fn a_plane_at_height_is_met_at_that_height() {
    let ray = WorldRay {
        origin: Vec3::new(0.0, 10.0, 0.0),
        direction: Vec3::new(1.0, -1.0, 0.0).normalize(),
    };
    let hit = ray.hits_horizontal_plane(4.0).unwrap();
    assert!((hit.y - 4.0).abs() < 1e-5, "got {hit:?}");
    assert!((hit.x - 6.0).abs() < 1e-5, "got {hit:?}");
}
