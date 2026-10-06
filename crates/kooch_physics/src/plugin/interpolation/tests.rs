//! 🔴 Arithmetic a smoke test cannot judge. A body drawn confidently at the wrong fraction of its
//! step looks exactly like one drawn right — the eye only reports "smooth" or "not".

use super::*;

fn at(x: f32) -> (Vec3, Quat) {
    (Vec3::new(x, 0.0, 0.0), Quat::IDENTITY)
}

fn entity() -> Entity {
    Entity::new(1, 0)
}

/// Halfway through a step is halfway between the two poses.
#[test]
fn a_half_step_is_halfway() {
    let mut poses = StepPoses::default();
    poses.stepped(entity(), at(0.0).0, at(0.0).1);
    poses.stepped(entity(), at(2.0).0, at(2.0).1);
    let (position, _) = poses.at(entity(), 0.5).expect("a recorded body");
    assert!((position.x - 1.0).abs() < 1e-5, "drawn at {position}");
}

/// 🔴 A body's FIRST step has nothing to come from, so it is drawn where it is rather than sliding
/// in from the origin — which is what a body spawned far from zero would do.
#[test]
fn a_first_step_does_not_slide_in() {
    let mut poses = StepPoses::default();
    poses.stepped(entity(), Vec3::new(100.0, 0.0, 0.0), Quat::IDENTITY);
    let (position, _) = poses.at(entity(), 0.0).expect("a recorded body");
    assert!((position.x - 100.0).abs() < 1e-5, "slid in from {position}");
}

/// 🔴 A jump no motion could produce is a teleport, and sliding across one smears the body through
/// whatever is between. An authored push is a cut by definition.
#[test]
fn a_teleport_is_a_cut() {
    let mut poses = StepPoses::default();
    poses.stepped(entity(), at(0.0).0, at(0.0).1);
    poses.stepped(entity(), at(50.0).0, at(50.0).1);
    let (position, _) = poses.at(entity(), 0.5).expect("a recorded body");
    assert!(
        (position.x - 50.0).abs() < 1e-5,
        "it slid to {position} instead of cutting",
    );
}

/// Two steps in one frame leave the frame drawn between the LAST two, not the first.
#[test]
fn only_the_last_two_steps_count() {
    let mut poses = StepPoses::default();
    for x in [0.0, 1.0, 2.0] {
        poses.stepped(entity(), at(x).0, at(x).1);
    }
    let (position, _) = poses.at(entity(), 0.0).expect("a recorded body");
    assert!(
        (position.x - 1.0).abs() < 1e-5,
        "came from {position}, not 1"
    );
}

/// A body that is gone does not keep a pose for whatever reuses its index.
#[test]
fn a_dropped_body_is_forgotten() {
    let mut poses = StepPoses::default();
    poses.stepped(entity(), at(1.0).0, at(1.0).1);
    poses.retain(&[]);
    assert!(poses.at(entity(), 0.5).is_none());
}

/// The short way round: `q` and `-q` are one rotation, and without matching them a body can spin
/// almost all the way round inside a single step.
#[test]
fn the_slerp_takes_the_short_way() {
    let from = Quat::IDENTITY;
    let to = -Quat::from_rotation_y(0.2);
    let quarter = short_slerp(from, to, 0.25);
    assert!(
        from.angle_between(quarter) < 0.1,
        "it went the long way: {}",
        from.angle_between(quarter),
    );
}
