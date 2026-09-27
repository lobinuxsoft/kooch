//! The rig with a [`CameraFraming`] (#1252): a target wandering inside the dead zone moves nothing.

use super::*;
use kooch_ecs::allocator::EntityAllocator;

/// A `Simple` vcam 5 m behind a target at the origin, framed with the defaults, and the target.
fn world() -> (Resources, Entity, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<CameraTarget>();
    registry.register_cpu_reflected::<CameraFraming>();
    registry.register_cpu_reflected::<CameraLookahead>();
    let (vcam, target) = (allocator.spawn(), allocator.spawn());
    let at = |position| Transform {
        position,
        ..Default::default()
    };
    registry
        .get_cpu_mut::<Transform>()
        .unwrap()
        .insert(vcam, at(Vec3::Z * 5.0));
    registry
        .get_cpu_mut::<Transform>()
        .unwrap()
        .insert(target, at(Vec3::ZERO));
    registry.get_cpu_mut::<VirtualCamera>().unwrap().insert(
        vcam,
        VirtualCamera {
            follow: crate::FOLLOW_SIMPLE,
            offset: Vec3::Z * 5.0,
            damping: false,
            ..Default::default()
        },
    );
    registry
        .get_cpu_mut::<CameraTarget>()
        .unwrap()
        .insert(target, CameraTarget::default());
    registry
        .get_cpu_mut::<CameraFraming>()
        .unwrap()
        .insert(vcam, CameraFraming::default());
    resources.insert(allocator);
    resources.insert(registry);
    (resources, vcam, target)
}

fn place(resources: &mut Resources, entity: Entity, position: Vec3) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<Transform>()
        .unwrap()
        .get_mut(entity)
        .unwrap()
        .position = position;
}

fn pose(resources: &Resources, entity: Entity) -> (Vec3, glam::Quat) {
    camera_pose(resources, entity).unwrap()
}

/// 🔴 #1323: the dead zone holds the **aim**, not the rig. The camera follows its target like any
/// other — framing is the stage that turns it, and a stage that moved it too was a second owner of
/// where the target lands on screen.
#[test]
fn the_dead_zone_holds_the_aim() {
    let (mut resources, vcam, target) = world();
    for _ in 0..10 {
        drive_virtual_cameras(&mut resources);
    }
    let (_, settled) = pose(&resources, vcam);
    // 60° at 5 m over 16:9 is ~10 m wide: the default dead zone reaches ~0.5 m either side.
    place(&mut resources, target, Vec3::X * 0.3);
    for _ in 0..30 {
        drive_virtual_cameras(&mut resources);
    }
    let (_, aimed) = pose(&resources, vcam);
    assert!(
        aimed.abs_diff_eq(settled, 1e-4),
        "the camera turned for a target inside the dead zone: {settled} then {aimed}",
    );
}

/// And leaving it moves the rig, until the target sits back on the zone's edge.
#[test]
fn leaving_the_dead_zone_moves_it() {
    let (mut resources, vcam, target) = world();
    drive_virtual_cameras(&mut resources);
    place(&mut resources, target, Vec3::X * 4.0);
    for _ in 0..200 {
        drive_virtual_cameras(&mut resources);
    }
    let (position, _) = pose(&resources, vcam);
    // It stops once the target is back on the dead zone's edge, not centred on it.
    assert!(position.x > 3.0 && position.x < 4.0, "{position}");
}

/// `screen` holds the target off centre, which moves the rig sideways rather than turning it: the
/// rotation belongs to whoever is looking around (#1329).
#[test]
fn the_screen_offset_moves_it() {
    let (mut resources, vcam, _) = world();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<CameraFraming>()
        .unwrap()
        .get_mut(vcam)
        .unwrap()
        .screen
        .x = 0.25;
    for _ in 0..200 {
        drive_virtual_cameras(&mut resources);
    }
    let (position, rotation) = pose(&resources, vcam);
    // Held right of centre, so the rig stands to the left of the target.
    assert!(position.x < -0.1, "{position}");
    // And it did not turn to do it.
    assert!(
        rotation.abs_diff_eq(glam::Quat::IDENTITY, 1e-3),
        "the frame turned the camera: {rotation}",
    );
}

/// With a lookahead and no framing, a running target puts the rig ahead of it (#1253).
#[test]
fn a_running_target_is_led() {
    let (mut resources, vcam, target) = world();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<CameraFraming>()
        .unwrap()
        .remove(vcam);
    registry.get_cpu_mut::<CameraLookahead>().unwrap().insert(
        vcam,
        CameraLookahead {
            smoothing_duration: 0.0,
            ..Default::default()
        },
    );
    let mut x = 0.0;
    for _ in 0..60 {
        x += 6.0 / 60.0;
        place(&mut resources, target, Vec3::X * x);
        drive_virtual_cameras(&mut resources);
    }
    let (position, _) = pose(&resources, vcam);
    // `Simple` sits on the led point plus its offset: 0.4 s at 6 m/s ahead.
    assert!(
        (position.x - (x + 2.4)).abs() < 1e-3,
        "{position} for a target at {x}"
    );
}

/// The rig's own position moves smoothly at any speed: every step within a factor of the one
/// before it.
///
/// 🔴 Without framing on purpose. A dead zone holds the camera still and then moves it — that is
/// what a dead zone IS, and a framed camera's position is meant to be piecewise. What must not
/// jump is the **frame**, which `framing_adds_no_jump_of_its_own` measures. The old design hid the
/// difference by carrying the target's velocity into the camera, which is the mechanism #1323
/// removed (#1329).
#[test]
fn the_rig_never_steps() {
    for speed in [6.0_f32, 40.0] {
        let (mut resources, vcam, target) = world();
        {
            let registry = resources.get_mut::<ComponentRegistry>().unwrap();
            let cam = registry
                .get_cpu_mut::<VirtualCamera>()
                .unwrap()
                .get_mut(vcam)
                .unwrap();
            cam.follow = crate::FOLLOW_THIRD_PERSON;
            cam.distance = 8.0;
            cam.pitch = 18.0;
            registry
                .get_cpu_mut::<CameraFraming>()
                .unwrap()
                .get_mut(vcam)
                .unwrap()
                .enabled = false;
        }
        let (dt, radius) = (1.0 / 60.0, 8.0);
        let (mut last, mut previous) = (Vec3::ZERO, 0.0_f32);
        for step in 0..240 {
            let angle = speed * dt * step as f32 / radius;
            place(
                &mut resources,
                target,
                Vec3::new(radius * angle.cos(), 0.0, radius * angle.sin()),
            );
            drive_virtual_cameras(&mut resources);
            let (position, _) = pose(&resources, vcam);
            let delta = (position - last).length();
            if step > 10 {
                assert!(
                    delta < previous * 1.6 + 1e-3 && delta > previous * 0.6 - 1e-3,
                    "step {step} at {speed} m/s moved {delta:.4} after {previous:.4}",
                );
            }
            previous = delta;
            last = position;
        }
    }
}

/// 🔴 #1323: the shake is on the **screen**, so that is where it has to be measured — and against
/// the rig alone, not against zero. A target orbiting at speed moves the frame by itself; the
/// claim is that framing adds no jump of its own, which is what the old design could not say.
#[test]
fn framing_adds_no_jump_of_its_own() {
    /// The largest CHANGE in how fast the target moves within the frame, radians per step per
    /// step. A jump is a change of rate; moving faster is not a jump.
    fn worst_step(framed: bool, speed: f32) -> f32 {
        let (mut resources, vcam, target) = world();
        {
            let registry = resources.get_mut::<ComponentRegistry>().unwrap();
            let cam = registry
                .get_cpu_mut::<VirtualCamera>()
                .unwrap()
                .get_mut(vcam)
                .unwrap();
            cam.follow = crate::FOLLOW_THIRD_PERSON;
            cam.distance = 8.0;
            cam.pitch = 18.0;
            cam.damping = true;
            cam.damping_duration = Vec3::splat(0.3);
            registry
                .get_cpu_mut::<CameraFraming>()
                .unwrap()
                .get_mut(vcam)
                .unwrap()
                .enabled = framed;
        }
        let (dt, radius) = (1.0 / 60.0, 8.0);
        let (mut last, mut previous, mut worst) = (0.0_f32, 0.0_f32, 0.0_f32);
        for step in 0..240 {
            let angle = speed * dt * step as f32 / radius;
            let at = Vec3::new(radius * angle.cos(), 0.0, radius * angle.sin());
            place(&mut resources, target, at);
            drive_virtual_cameras(&mut resources);
            let (position, rotation) = pose(&resources, vcam);
            let Some(to_target) = (at - position).try_normalize() else {
                continue;
            };
            let framed = (rotation * -Vec3::Z).angle_between(to_target);
            let delta = framed - last;
            if step > 20 {
                worst = worst.max((delta - previous).abs());
            }
            previous = delta;
            last = framed;
        }
        worst
    }

    for speed in [6.0_f32, 40.0] {
        let bare = worst_step(false, speed);
        let framed = worst_step(true, speed);
        assert!(
            framed <= bare * 1.25 + 1e-3,
            "at {speed} m/s framing changed the frame's rate by {framed:.5} against the rig's own {bare:.5}",
        );
    }
}
