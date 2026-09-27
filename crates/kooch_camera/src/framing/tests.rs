use super::*;

const DT: f32 = 1.0 / 60.0;

/// 90° over a square screen: at 1 m depth the screen is 2 m by 2 m, so fractions read as metres/2.
fn lens() -> Lens {
    Lens::new(90.0, 1.0)
}

fn framing() -> CameraFraming {
    CameraFraming {
        dead_zone: Vec2::splat(0.2),
        soft_zone: Vec2::splat(0.6),
        ..Default::default()
    }
}

/// The camera stands at the origin looking down −Z, which is where every target below is placed
/// from.
const EYE: Vec3 = Vec3::ZERO;

/// Where `target` lands on screen under `rotation`, as a fraction of the screen from its centre.
fn on_screen(rotation: Quat, target: Vec3) -> Vec2 {
    let offset = target - EYE;
    let depth = offset.dot(rotation * -Vec3::Z);
    let span = lens().span(depth);
    Vec2::new(
        offset.dot(rotation * Vec3::X) / span.x,
        offset.dot(rotation * Vec3::Y) / span.y,
    )
}

/// One step of aiming at `target`, from a camera looking down −Z.
fn step(framing: &CameraFraming, target: Vec3) -> Quat {
    let mut state = Framed::at(target);
    framing.aim(
        &mut state,
        EYE,
        Quat::IDENTITY,
        target,
        Vec3::Y,
        Vec3::Y,
        lens(),
        DT,
    )
}

/// Inside the dead zone the camera holds still: the rotation it was given is the rotation it keeps.
#[test]
fn inside_the_dead_zone_holds() {
    // 4 m down −Z, a little off centre: 0.05 of the screen, inside the 0.1 half-width.
    let target = Vec3::new(0.35, -0.35, -4.0);
    let aimed = step(&framing(), target);
    assert!(
        aimed.abs_diff_eq(Quat::IDENTITY, 1e-4),
        "the camera turned for a target inside the dead zone: {aimed}",
    );
}

/// Outside it, the camera turns towards the target — part of the way in one step, not all of it.
#[test]
fn the_soft_zone_eases_back() {
    let target = Vec3::new(2.0, 0.0, -4.0);
    let before = on_screen(Quat::IDENTITY, target).x;
    let after = on_screen(step(&framing(), target), target).x;
    assert!(
        after < before,
        "it did not turn at all: {before} then {after}"
    );
    assert!(
        after > 0.1,
        "it snapped to the dead zone's edge in one step: {after}",
    );
}

/// 🔴 #1323: the soft zone RAMPS the correction in — none at the dead zone's edge, all of it at the
/// soft one. A dead zone without that band switches the camera from not turning at all to turning
/// at full rate between two frames, which is a step in the frame's speed and the shake itself. It
/// is not a wall: a wall shoves the ease's answer, and that measured three times worse.
#[test]
fn the_soft_zone_ramps_the_correction() {
    let rigid = CameraFraming {
        soft_duration: 0.0,
        ..framing()
    };
    // Just outside the dead zone (0.1): almost nothing is owed yet.
    let near = Vec3::new(2.2, 0.0, -10.0);
    let moved_near = {
        let before = on_screen(Quat::IDENTITY, near).x;
        before - on_screen(step(&rigid, near), near).x
    };
    // Out at the soft zone's edge (0.3): all of it.
    let far = Vec3::new(6.0, 0.0, -10.0);
    let moved_far = {
        let before = on_screen(Quat::IDENTITY, far).x;
        before - on_screen(step(&rigid, far), far).x
    };
    assert!(
        moved_near < 0.02,
        "the correction switched on at the dead edge: {moved_near}",
    );
    assert!(
        moved_far > 0.15,
        "the correction never reached full at the soft edge: {moved_far}",
    );
}

/// A rigid ease lands on the dead zone's edge in one step, which is what zero means. With no band
/// there is nothing to ramp, so the whole correction applies at once.
#[test]
fn a_rigid_ease_reaches_the_dead_edge() {
    let rigid = CameraFraming {
        soft_duration: 0.0,
        soft_zone: Vec2::splat(0.2),
        ..framing()
    };
    let target = Vec3::new(2.0, 0.0, -4.0);
    let after = on_screen(step(&rigid, target), target).x;
    assert!((after - 0.1).abs() < 1e-3, "landed at {after}, wanted 0.1");
}

/// `screen` moves where the target is held, so a rig can keep it off centre.
#[test]
fn the_screen_offset_is_where_it_holds() {
    let right = CameraFraming {
        screen: Vec2::new(0.25, 0.0),
        dead_zone: Vec2::ZERO,
        soft_zone: Vec2::ZERO,
        soft_duration: 0.0,
        ..Default::default()
    };
    let target = Vec3::new(0.0, 0.0, -4.0);
    let after = on_screen(step(&right, target), target).x;
    assert!((after - 0.25).abs() < 1e-3, "held at {after}, wanted 0.25");
}

/// 🔴 The duration is a duration: once the target stops, the camera brings it to the dead zone's
/// edge in exactly `soft_duration`, at any frame rate.
#[test]
fn the_soft_zone_arrives_on_time() {
    let framing = CameraFraming {
        soft_duration: 0.5,
        ..framing()
    };
    let target = Vec3::new(2.0, 0.0, -4.0);
    for fps in [30.0_f32, 60.0, 144.0] {
        // Walked there rather than teleported: a target that appears somewhere is not a speed, and
        // the arrival is measured from the moment it stops.
        let run = |seconds: f32| {
            let mut state = Framed::at(Vec3::new(0.0, 0.0, -4.0));
            let mut aimed = Quat::IDENTITY;
            let walk = (0.25 * fps).round() as usize;
            let mut at = |state: &mut Framed, point: Vec3, aimed: Quat| {
                framing.aim(
                    state,
                    EYE,
                    aimed,
                    point,
                    Vec3::Y,
                    Vec3::Y,
                    lens(),
                    1.0 / fps,
                )
            };
            for step in 0..walk {
                let point = Vec3::new(target.x * (step as f32 + 1.0) / walk as f32, 0.0, target.z);
                aimed = at(&mut state, point, aimed);
            }
            for _ in 0..(seconds * fps).round() as usize {
                aimed = at(&mut state, target, aimed);
            }
            on_screen(aimed, target).x
        };
        assert!(
            (run(0.5) - 0.1).abs() < 5e-3,
            "{fps} fps: {} rather than the dead edge",
            run(0.5),
        );
        assert!(run(0.4) > 0.1 + 1e-3, "{fps} fps arrived early");
    }
}

/// A target behind the camera has no screen to be framed on, and inventing one throws the pose.
#[test]
fn a_target_behind_is_left_alone() {
    let behind = Vec3::new(0.0, 0.0, 4.0);
    let aimed = step(&framing(), behind);
    assert!(aimed.abs_diff_eq(Quat::IDENTITY, 1e-4), "{aimed}");
}
