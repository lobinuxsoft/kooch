use super::*;

use glam::Quat;

use crate::frame::CameraFrame;

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

/// Nothing held off centre, nothing eased: what a test that wants the whole correction in one step
/// asks for.
fn rigid() -> CameraFraming {
    CameraFraming {
        dead_zone: Vec2::ZERO,
        soft_zone: Vec2::ZERO,
        soft_time: 0.0,
        ..Default::default()
    }
}

/// The camera stands four metres back and looks down −Z.
const EYE: Vec3 = Vec3::new(0.0, 0.0, 4.0);

/// A frame at `EYE` looking down −Z, with `lead` metres of lead already folded into where the target
/// is held — as the plugin works it out.
fn frame_at(target: Vec3, lead: Vec3) -> CameraFrame {
    let mut frame = CameraFrame::new(EYE, EYE, Quat::IDENTITY, target, lens());
    let span = lens().span((target - EYE).z.abs().max(0.01));
    frame.screen = -Vec2::new(lead.x / span.x, lead.y / span.y);
    frame
}

/// Where `target` lands on screen for a camera at `eye` turned by `rotation`.
fn on_screen(rotation: Quat, eye: Vec3, target: Vec3) -> Vec2 {
    let to = target - eye;
    let (right, above, forward) = (
        rotation * Vec3::X,
        rotation * Vec3::Y,
        rotation * Vec3::NEG_Z,
    );
    let span = lens().span(to.dot(forward));
    Vec2::new(to.dot(right) / span.x, to.dot(above) / span.y)
}

/// One step, answering where the target ends up on screen.
fn step(framing: &CameraFraming, state: &mut Framed, target: Vec3, dt: f32) -> Vec2 {
    let mut frame = frame_at(target, Vec3::ZERO);
    framing.compose(state, &mut frame, Vec3::Y, Vec3::Z, dt);
    on_screen(frame.rotation, frame.position, target)
}

/// Inside the dead zone the camera keeps the orientation it has: it does not chase a target that has
/// not gone anywhere worth answering.
#[test]
fn inside_the_dead_zone_holds() {
    let mut state = Framed::at(Vec3::ZERO);
    // 0.05 of the screen: inside the 0.1 half-width.
    let target = Vec3::new(0.4, 0.0, 0.0);
    let seen = step(&framing(), &mut state, target, DT);
    assert!(
        (seen.x - 0.05).abs() < 0.01,
        "the camera turned for a target inside the dead zone: {seen}",
    );
}

/// Outside it the camera follows, part of the way in one step.
#[test]
fn the_soft_zone_eases_back() {
    let mut state = Framed::at(Vec3::ZERO);
    let target = Vec3::new(3.0, 0.0, 0.0);
    let seen = step(&framing(), &mut state, target, DT).x;
    let before = on_screen(Quat::IDENTITY, EYE, target).x;
    assert!(seen < before, "it did not follow at all: {seen}");
    assert!(seen > 0.1, "it snapped all the way to the edge: {seen}");
}

/// 🔴 #1361: the composer pans and tilts the camera where it is. Where the camera stands is the
/// body's, walls included, and a frame that moved it is what could not coexist with a shoulder.
#[test]
fn the_position_belongs_to_the_body() {
    let mut frame = frame_at(Vec3::new(3.0, 1.0, 0.0), Vec3::ZERO);
    let mut state = Framed::at(Vec3::ZERO);
    framing().compose(&mut state, &mut frame, Vec3::Y, Vec3::Z, DT);
    assert!(frame.position.abs_diff_eq(EYE, 1e-6), "{}", frame.position);
    assert!(frame.free.abs_diff_eq(EYE, 1e-6), "{}", frame.free);
}

/// `screen` moves where the target is held, so a rig can keep it off centre.
#[test]
fn the_screen_offset_is_where_it_holds() {
    let right = CameraFraming {
        screen: Vec2::new(0.25, 0.0),
        ..rigid()
    };
    let mut state = Framed::at(Vec3::ZERO);
    let seen = step(&right, &mut state, Vec3::ZERO, DT).x;
    assert!((seen - 0.25).abs() < 0.02, "held at {seen}, wanted 0.25");
}

/// The horizon stays level: a pan is about the vcam's own up and never rolls the view, which is why
/// the order in `applied` is pan first.
#[test]
fn a_pan_never_rolls_the_horizon() {
    let mut state = Framed::at(Vec3::ZERO);
    let mut frame = frame_at(Vec3::new(4.0, 2.0, 0.0), Vec3::ZERO);
    rigid().compose(&mut state, &mut frame, Vec3::Y, Vec3::Z, DT);
    let right = frame.rotation * Vec3::X;
    assert!(
        right.dot(Vec3::Y).abs() < 1e-4,
        "the camera rolled: right is {right}",
    );
}

/// 🔴 The duration is a duration: once the target stops, the camera brings it to the dead zone's
/// edge in exactly `soft_time`, at any frame rate.
#[test]
fn the_soft_zone_arrives_on_time() {
    let framing = CameraFraming {
        soft_time: 0.5,
        ..framing()
    };
    for fps in [30.0_f32, 60.0, 144.0] {
        let run = |seconds: f32| {
            let dt = 1.0 / fps;
            let walk = (0.25 * fps).round() as usize;
            let target = Vec3::new(3.0, 0.0, 0.0);
            let mut state = Framed::at(Vec3::ZERO);
            let mut rotation = Quat::IDENTITY;
            let mut at = |state: &mut Framed, point: Vec3, rotation: Quat| {
                let mut frame = CameraFrame::new(EYE, EYE, rotation, point, lens());
                framing.compose(state, &mut frame, Vec3::Y, Vec3::Z, dt);
                frame.rotation
            };
            for step in 0..walk {
                let point = Vec3::new(target.x * (step as f32 + 1.0) / walk as f32, 0.0, 0.0);
                rotation = at(&mut state, point, rotation);
            }
            for _ in 0..(seconds * fps).round() as usize {
                rotation = at(&mut state, target, rotation);
            }
            on_screen(rotation, EYE, target).x
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

/// 🔴 A target behind the camera is turned towards, which the position model could not do: it had no
/// screen to measure a fraction on and had to leave the pose alone. An angle has no such hole.
#[test]
fn a_target_behind_is_turned_to() {
    let mut state = Framed::at(Vec3::ZERO);
    let behind = Vec3::new(0.0, 0.0, 8.0);
    let mut frame = frame_at(behind, Vec3::ZERO);
    rigid().compose(&mut state, &mut frame, Vec3::Y, Vec3::Z, DT);
    let forward = frame.rotation * Vec3::NEG_Z;
    assert!(
        forward.dot(Vec3::Z) > 0.99,
        "it did not turn around: {forward}",
    );
}

/// 🔴 #1330: a lead moves where the target is HELD, not what is framed. Leading a runner means
/// showing what is ahead of them, which is the same as holding them behind centre — and the dead
/// zone travels with it instead of fighting it.
#[test]
fn a_lead_moves_where_it_holds() {
    let target = Vec3::ZERO;
    let mut state = Framed::at(target);
    // Two metres of lead along +X: the target is held that much to the LEFT of centre.
    let mut frame = frame_at(target, Vec3::X * 2.0);
    rigid().compose(&mut state, &mut frame, Vec3::Y, Vec3::Z, DT);
    let seen = on_screen(frame.rotation, frame.position, target).x;
    assert!(seen < -0.1, "the lead did not move the aim: {seen}");

    // And with no lead it sits in the middle.
    let mut plain = Framed::at(target);
    let still = step(&rigid(), &mut plain, target, DT).x;
    assert!(still.abs() < 0.01, "{still}");
}

/// 🔴 #1363: a composer turns the orientation it already has, so a roll it picks up is kept. Under
/// an `up` that moves — a ball rolling around a planet — one is picked up every frame, and the
/// horizon ends up at 156°. `a_pan_never_rolls_the_horizon` holds `up` still and cannot see it.
#[test]
fn a_moving_up_never_rolls_it() {
    let framing = framing();
    let mut state = Framed::at(Vec3::ZERO);
    let mut rotation = Quat::IDENTITY;
    let mut up = Vec3::Y;
    for step in 0..600 {
        // The up walks a whole turn, as gravity does around a planet.
        let angle = step as f32 * std::f32::consts::TAU / 600.0;
        up = Vec3::new(angle.sin(), angle.cos(), 0.0);
        let target = Vec3::new(angle.cos() * 2.0, 0.0, angle.sin());
        let mut frame = CameraFrame::new(EYE, EYE, rotation, target, lens());
        framing.compose(&mut state, &mut frame, up, Vec3::Z, DT);
        rotation = frame.rotation;
    }
    let right = rotation * Vec3::X;
    assert!(
        right.dot(up).abs() < 1e-3,
        "the horizon rolled to {} off {up}",
        right.dot(up),
    );
}
