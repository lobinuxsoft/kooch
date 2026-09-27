use super::*;

fn lens() -> Lens {
    Lens::new(90.0, 1.0)
}

/// The frame carries where the body wanted to stand, so a stage that is pushed still knows.
#[test]
fn a_displaced_frame_remembers_where_it_wanted() {
    let mut frame = CameraFrame::new(
        Vec3::new(0.0, 0.0, 4.0),
        Quat::IDENTITY,
        Vec3::ZERO,
        Vec3::Y,
        Vec3::Y,
        lens(),
    );
    frame.displace(Vec3::new(2.0, 0.0, 4.0));
    assert_eq!(frame.position, Vec3::new(2.0, 0.0, 4.0));
    assert_eq!(frame.free, Vec3::new(0.0, 0.0, 4.0));
}

/// A target dead ahead sits in the middle, and one off to the side does not.
#[test]
fn what_it_sees_is_where_it_is() {
    let frame = CameraFrame::new(
        Vec3::new(0.0, 0.0, 4.0),
        Quat::IDENTITY,
        Vec3::ZERO,
        Vec3::Y,
        Vec3::Y,
        lens(),
    );
    assert_eq!(frame.seen(Vec3::ZERO), Some(Vec2::ZERO));
    let right = frame.seen(Vec3::new(4.0, 0.0, 0.0)).unwrap();
    assert!(right.x > 0.0, "{right}");
}

/// Behind the camera there is no screen to be on.
#[test]
fn nothing_behind_is_seen() {
    let frame = CameraFrame::new(
        Vec3::new(0.0, 0.0, 4.0),
        Quat::IDENTITY,
        Vec3::ZERO,
        Vec3::Y,
        Vec3::Y,
        lens(),
    );
    assert_eq!(frame.seen(Vec3::new(0.0, 0.0, 9.0)), None);
}
