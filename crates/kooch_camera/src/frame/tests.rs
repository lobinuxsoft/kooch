use super::*;

fn lens() -> Lens {
    Lens::new(90.0, 1.0)
}

/// The frame carries where the body wanted to stand, so a stage that is pushed still knows.
#[test]
fn a_displaced_frame_remembers_where_it_wanted() {
    let mut frame = CameraFrame::new(
        Vec3::new(0.0, 0.0, 4.0),
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
        Vec3::new(0.0, 0.0, 4.0),
        Quat::IDENTITY,
        Vec3::ZERO,
        Vec3::Y,
        Vec3::Y,
        lens(),
    );
    assert_eq!(frame.seen(Vec3::new(0.0, 0.0, 9.0)), None);
}

/// 🔴 #1331: the rule the stages exist to keep. A stage that moves the camera must not touch where
/// the body wanted it, or the next stage reads its own displacement as something to answer — which
/// is how a wall's push became slack the frame spent a `soft_duration` undoing.
#[test]
fn displacing_never_moves_the_bodys_answer() {
    let body = Vec3::new(0.0, 2.0, 8.0);
    let mut frame = CameraFrame::new(
        body,
        body,
        Quat::IDENTITY,
        Vec3::ZERO,
        Vec3::Y,
        Vec3::Y,
        lens(),
    );
    // A frame moves it sideways, then a wall pulls it in.
    frame.displace(frame.position + Vec3::X * 1.5);
    frame.displace(frame.position - Vec3::Z * 3.0);
    assert_eq!(frame.free, body, "a stage moved the body's answer");
    assert_eq!(frame.previous, body, "a stage moved where the camera was");
}
