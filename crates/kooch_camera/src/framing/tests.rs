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

/// The rig looks down −Z from four metres back, which is `wanted` unless a test moves it.
const BACK: Vec3 = Vec3::new(0.0, 0.0, 4.0);

/// Where `target` lands on screen for a camera at `eye` looking down −Z.
fn on_screen(eye: Vec3, target: Vec3) -> Vec2 {
    let offset = target - eye;
    let span = lens().span(offset.z.abs());
    Vec2::new(offset.x / span.x, offset.y / span.y)
}

/// One step of following, the rig wanting to sit at `wanted` and the camera standing at `at`.
fn step(framing: &CameraFraming, state: &mut Framed, wanted: Vec3, at: Vec3, target: Vec3) -> Vec3 {
    framing.follow(state, wanted, at, Quat::IDENTITY, target, lens(), DT)
}

/// Inside the dead zone the rig keeps the position it has: the camera does not chase a target that
/// has not gone anywhere worth answering.
#[test]
fn inside_the_dead_zone_holds() {
    let mut state = Framed::at(Vec3::ZERO);
    // Settle first.
    let settled = step(&framing(), &mut state, BACK, BACK, Vec3::ZERO);
    // 0.05 of the screen: inside the 0.1 half-width.
    let target = Vec3::new(0.4, 0.0, 0.0);
    let after = step(
        &framing(),
        &mut state,
        BACK + Vec3::X * 0.4,
        settled,
        target,
    );
    assert!(
        (after.x - settled.x).abs() < 0.01,
        "the rig moved for a target inside the dead zone: {settled} then {after}",
    );
}

/// Outside it the rig follows, part of the way in one step.
#[test]
fn the_soft_zone_eases_back() {
    let mut state = Framed::at(Vec3::ZERO);
    let settled = step(&framing(), &mut state, BACK, BACK, Vec3::ZERO);
    let target = Vec3::new(3.0, 0.0, 0.0);
    let after = step(
        &framing(),
        &mut state,
        BACK + Vec3::X * 3.0,
        settled,
        target,
    );
    assert!(after.x > 0.0, "it did not follow at all: {after}");
    assert!(after.x < 3.0, "it snapped all the way: {after}");
}

/// 🔴 #1329: only the screen's axes. How far away the camera sits is the rig's business — a frame
/// that pushed along the forward would fight the arm and the wall alike.
#[test]
fn the_depth_belongs_to_the_rig() {
    let mut state = Framed::at(Vec3::ZERO);
    let target = Vec3::new(3.0, 1.0, 0.0);
    let after = step(&framing(), &mut state, BACK, BACK, target);
    assert!(
        (after.z - BACK.z).abs() < 1e-4,
        "the frame moved the camera in depth: {after}",
    );
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
    let mut state = Framed::at(Vec3::ZERO);
    let target = Vec3::ZERO;
    let after = step(&right, &mut state, BACK, BACK, target);
    let seen = on_screen(after, target).x;
    assert!((seen - 0.25).abs() < 0.02, "held at {seen}, wanted 0.25");
}

/// 🔴 The duration is a duration: once the target stops, the rig brings it to the dead zone's edge
/// in exactly `soft_duration`, at any frame rate.
#[test]
fn the_soft_zone_arrives_on_time() {
    let framing = CameraFraming {
        soft_duration: 0.5,
        ..framing()
    };
    for fps in [30.0_f32, 60.0, 144.0] {
        let run = |seconds: f32| {
            let mut state = Framed::at(Vec3::ZERO);
            let dt = 1.0 / fps;
            let walk = (0.25 * fps).round() as usize;
            let target = Vec3::new(3.0, 0.0, 0.0);
            let mut eye = BACK;
            let mut at = |state: &mut Framed, point: Vec3, eye: Vec3| {
                framing.follow(
                    state,
                    BACK + Vec3::X * point.x,
                    eye,
                    Quat::IDENTITY,
                    point,
                    lens(),
                    dt,
                )
            };
            for step in 0..walk {
                let point = Vec3::new(target.x * (step as f32 + 1.0) / walk as f32, 0.0, 0.0);
                eye = at(&mut state, point, eye);
            }
            for _ in 0..(seconds * fps).round() as usize {
                eye = at(&mut state, target, eye);
            }
            on_screen(eye, target).x
        };
        // On the dead zone's edge: 0.1 of the screen.
        assert!(
            (run(0.5) - 0.1).abs() < 0.02,
            "{fps} fps: {} rather than the dead edge",
            run(0.5),
        );
        assert!(run(0.4) > 0.1 + 1e-3, "{fps} fps arrived early");
    }
}

/// A target behind the camera has no screen to be framed on, and the rig's own answer stands.
#[test]
fn a_target_behind_is_left_alone() {
    let mut state = Framed::at(Vec3::ZERO);
    let behind = Vec3::new(0.0, 0.0, 8.0);
    let after = step(&framing(), &mut state, BACK, BACK, behind);
    assert!(after.abs_diff_eq(BACK, 1e-4), "{after}");
}
