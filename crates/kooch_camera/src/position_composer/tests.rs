use super::*;

use glam::Quat;

/// 90° over a square screen: at 1 m depth the screen is 2 m by 2 m, so fractions read as metres/2.
fn lens() -> crate::framing::Lens {
    crate::framing::Lens::new(90.0, 1.0)
}

fn composer() -> PositionComposer {
    PositionComposer {
        camera_distance: 10.0,
        dead_zone: Vec2::splat(0.2),
        soft_zone: Vec2::splat(0.2),
        damping_value: Vec3::ZERO,
        center_on_activate: false,
        ..Default::default()
    }
}

/// A camera at `at` looking down −Z, with the target at `target`.
fn frame(at: Vec3, target: Vec3) -> CameraFrame {
    CameraFrame::new(at, at, Quat::IDENTITY, target, lens())
}

/// Where `target` lands on screen for a camera at `at` looking down −Z.
fn on_screen(at: Vec3, target: Vec3) -> Vec2 {
    let to = target - at;
    let span = lens().span(-to.z);
    Vec2::new(to.x / span.x, to.y / span.y)
}

/// The whole of the Body: the camera slides along its own forward until the target is the distance
/// away it asked for.
#[test]
fn the_depth_is_answered_first() {
    let placed = composer().placed(&frame(Vec3::Z * 4.0, Vec3::ZERO), false, 1.0 / 60.0);
    assert!((placed.z - 10.0).abs() < 1e-3, "{placed:?}");
    assert!(placed.x.abs() < 1e-3 && placed.y.abs() < 1e-3, "{placed:?}");
}

/// And a depth inside the dead zone is not answered at all.
#[test]
fn a_depth_inside_its_zone_holds() {
    let composer = PositionComposer {
        dead_zone_depth: 4.0,
        ..composer()
    };
    let at = Vec3::Z * 9.0;
    let placed = composer.placed(&frame(at, Vec3::ZERO), false, 1.0 / 60.0);
    assert!(placed.abs_diff_eq(at, 1e-3), "{placed:?}");
}

/// Inside the dead zone the camera holds still: that is what a dead zone is.
#[test]
fn inside_the_dead_zone_holds() {
    let at = Vec3::Z * 10.0;
    // 0.05 of the screen: inside the 0.1 half-width.
    let target = Vec3::X * 1.0;
    let placed = composer().placed(&frame(at, target), false, 1.0 / 60.0);
    assert!(placed.abs_diff_eq(at, 1e-3), "{placed:?}");
}

/// Outside it the camera **slides** — it never turns, which is the whole difference from the
/// rotation composer (#1361).
#[test]
fn leaving_the_dead_zone_slides_it() {
    let at = Vec3::Z * 10.0;
    let target = Vec3::X * 8.0;
    let mut frame = frame(at, target);
    let placed = composer().placed(&frame, false, 1.0 / 60.0);
    frame.place(placed);
    assert_eq!(frame.rotation, Quat::IDENTITY, "the body turned the camera");
    // It stops once the target is back on the dead zone's edge, not centred on it.
    let seen = on_screen(placed, target).x;
    assert!((seen - 0.1).abs() < 0.01, "held at {seen}, not the edge");
}

/// `screen_position` moves where the target is held, and the camera stands aside to do it.
#[test]
fn the_screen_offset_slides_it() {
    let composer = PositionComposer {
        screen_position: Vec2::new(0.25, 0.0),
        dead_zone: Vec2::ZERO,
        soft_zone: Vec2::ZERO,
        ..composer()
    };
    let placed = composer.placed(&frame(Vec3::Z * 10.0, Vec3::ZERO), false, 1.0 / 60.0);
    let seen = on_screen(placed, Vec3::ZERO).x;
    assert!((seen - 0.25).abs() < 0.01, "held at {seen}, wanted 0.25");
}

/// 🔴 The point of it being a Body: the axes are the **camera's**, whose up is the vcam's. On the
/// side of a planet it slides along the local horizon, not world X.
#[test]
fn it_slides_on_the_local_horizon() {
    // Up is +X: the camera stands along +Y looking down −Y, its own up pointing along +X.
    let up = Vec3::X;
    let at = Vec3::Y * 10.0;
    let rotation = Quat::from_mat3(&glam::Mat3::from_cols(
        Vec3::Z, // right: forward × up, with forward −Y
        up,      // up
        Vec3::Y, // -forward
    ));
    let target = up * 6.0;
    let frame = CameraFrame::new(at, at, rotation, target, lens());
    let placed = composer().placed(&frame, false, 1.0 / 60.0);
    let moved = placed - at;
    assert!(
        moved.dot(up) > 0.5,
        "it should have slid along the local up, got {moved:?}",
    );
    assert!(moved.x.abs() > moved.z.abs(), "{moved:?}");
}

/// Taking over lands on the answer instead of easing in from wherever the camera was left.
#[test]
fn arriving_centres_at_once() {
    let composer = PositionComposer {
        damping_value: Vec3::splat(0.5),
        center_on_activate: true,
        ..composer()
    };
    let at = Vec3::Z * 4.0;
    let arriving = composer.placed(&frame(at, Vec3::ZERO), true, 1.0 / 60.0);
    assert!((arriving.z - 10.0).abs() < 1e-3, "{arriving:?}");
    let settled = composer.placed(&frame(at, Vec3::ZERO), false, 1.0 / 60.0);
    assert!(
        settled.z < 6.0,
        "a running composer should ease: {settled:?}"
    );
}
