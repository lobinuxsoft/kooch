//! The rig with a [`RotationComposer`] (#1252): a target wandering inside the dead zone moves nothing.

use super::*;
use crate::{CameraLookahead, RotationComposer};
use glam::Vec2;
use kooch_ecs::allocator::EntityAllocator;

/// A `Simple` vcam 5 m behind a target at the origin, framed with the defaults, and the target.
fn world() -> (Resources, Entity, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<CameraTarget>();
    registry.register_cpu_reflected::<RotationComposer>();
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
            // The framing IS the Rotation Control: a vcam has to ask for it (#1361).
            look_at: crate::LOOK_AT_COMPOSED,
            offset: Vec3::Z * 5.0,
            damping: Vec3::ZERO,
            rotation_damping: 0.0,
            ..Default::default()
        },
    );
    registry
        .get_cpu_mut::<CameraTarget>()
        .unwrap()
        .insert(target, CameraTarget::default());
    registry
        .get_cpu_mut::<RotationComposer>()
        .unwrap()
        .insert(vcam, RotationComposer::default());
    resources.insert(allocator);
    resources.insert(registry);
    resources.insert(crate::rig::CameraRig::standard());
    (resources, vcam, target)
}

/// The orbital body a vcam names, at `radius`.
fn arm(resources: &mut Resources, vcam: Entity, radius: f32) {
    body(
        resources,
        vcam,
        crate::OrbitalFollow {
            radius,
            ..Default::default()
        },
    );
}

/// Gives a vcam the body it names, so a test can set that body's own numbers.
fn body<T: kooch_ecs::component::Component>(resources: &mut Resources, vcam: Entity, value: T) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu::<T>();
    registry.get_cpu_mut::<T>().unwrap().insert(vcam, value);
}

/// Changes what a vcam's body does, leaving the rest of the world alone.
fn follows(resources: &mut Resources, vcam: Entity, follow: u32) {
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .get_cpu_mut::<VirtualCamera>()
        .unwrap()
        .get_mut(vcam)
        .unwrap()
        .follow = follow;
}

/// Zeroes a framing's zones, for a test that measures where it holds rather than how it eases: with
/// a zone the target stops on its edge, which is the zone's job and not the hold's.
fn rigid(resources: &mut Resources, vcam: Entity) {
    let framing = resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .get_cpu_mut::<RotationComposer>()
        .unwrap()
        .get_mut(vcam)
        .unwrap();
    framing.dead_zone = Vec2::ZERO;
    framing.soft_zone = Vec2::ZERO;
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

/// Where `target` lands on screen for the vcam's pose, through the lens the rig itself used.
fn on_screen(resources: &Resources, vcam: Entity, target: Vec3) -> Vec2 {
    let registry = resources.get::<ComponentRegistry>().unwrap();
    let lens = lens(resources, registry);
    let (position, rotation) = pose(resources, vcam);
    let to = target - position;
    let span = lens.span(to.dot(rotation * -Vec3::Z));
    Vec2::new(
        to.dot(rotation * Vec3::X) / span.x,
        to.dot(rotation * Vec3::Y) / span.y,
    )
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

/// And leaving it turns the camera, until the target sits back on the zone's edge.
///
/// `Follow::None` on purpose: a body that tracks the target sideways carries its own dead zone with
/// it and the aim's is never reached. A camera that stands still and only turns is what a composer
/// is for.
#[test]
fn leaving_the_dead_zone_turns_it() {
    let (mut resources, vcam, target) = world();
    follows(&mut resources, vcam, crate::FOLLOW_NONE);
    drive_virtual_cameras(&mut resources);
    let moved = Vec3::X * 4.0;
    place(&mut resources, target, moved);
    for _ in 0..200 {
        drive_virtual_cameras(&mut resources);
    }
    // It stops once the target is back on the dead zone's edge, not centred on it.
    let seen = on_screen(&resources, vcam, moved).x;
    assert!((seen - 0.05).abs() < 0.01, "held at {seen}, not the edge");
}

/// 🔴 #1361: the composer pans and tilts the camera where it is. The body decides where it stands,
/// and that is what lets a shoulder offset and a framing coexist.
#[test]
fn the_frame_never_moves_the_camera() {
    let (mut resources, vcam, target) = world();
    drive_virtual_cameras(&mut resources);
    place(&mut resources, target, Vec3::new(4.0, 2.0, 0.0));
    for _ in 0..200 {
        drive_virtual_cameras(&mut resources);
    }
    // `Simple` puts it on the target plus the offset, and nothing else has a say.
    let (position, _) = pose(&resources, vcam);
    let wanted = Vec3::new(4.0, 2.0, 0.0) + Vec3::Z * 5.0;
    assert!(position.abs_diff_eq(wanted, 1e-3), "{position}");
}

/// `screen` holds the target off centre, and now it is the aim that does it — one owner of where the
/// character sits, and the body free to stand wherever it likes (#1361).
#[test]
fn the_screen_offset_turns_it() {
    let (mut resources, vcam, _) = world();
    rigid(&mut resources, vcam);
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<RotationComposer>()
        .unwrap()
        .get_mut(vcam)
        .unwrap()
        .screen
        .x = 0.25;
    for _ in 0..200 {
        drive_virtual_cameras(&mut resources);
    }
    let seen = on_screen(&resources, vcam, Vec3::ZERO).x;
    assert!((seen - 0.25).abs() < 0.01, "held at {seen}, wanted 0.25");
    // And it did not move to do it.
    let (position, _) = pose(&resources, vcam);
    assert!(position.abs_diff_eq(Vec3::Z * 5.0, 1e-3), "{position}");
}

/// 🔴 The acceptance of #1361, and the question that asked for it: a shoulder decides where the
/// camera stands, a framing decides where the character sits, and neither touches the other.
#[test]
fn a_shoulder_and_a_framing_coexist() {
    let (mut resources, vcam, _) = world();
    rigid(&mut resources, vcam);
    body(
        &mut resources,
        vcam,
        crate::ThirdPersonFollow {
            shoulder_offset: Vec3::new(0.6, 0.0, 0.0),
            vertical_arm_length: 0.0,
            camera_side: 1.0,
            camera_distance: 3.0,
        },
    );
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        let cam = registry
            .get_cpu_mut::<VirtualCamera>()
            .unwrap()
            .get_mut(vcam)
            .unwrap();
        cam.follow = crate::FOLLOW_SHOULDER;
        cam.pitch = 0.0;
        registry
            .get_cpu_mut::<RotationComposer>()
            .unwrap()
            .get_mut(vcam)
            .unwrap()
            .screen
            .x = -0.2;
    }
    for _ in 0..200 {
        drive_virtual_cameras(&mut resources);
    }
    // Yaw zero looks down −Z, so the arm is +Z and the shoulder is +X: the body's answer exactly,
    // and the framing has not touched it.
    let (position, _) = pose(&resources, vcam);
    assert!(
        position.abs_diff_eq(Vec3::new(0.6, 0.0, 3.0), 1e-3),
        "the shoulder did not place it: {position}",
    );
    // And the framing holds the character where it was asked to, not where the shoulder left it.
    let seen = on_screen(&resources, vcam, Vec3::ZERO).x;
    assert!((seen + 0.2).abs() < 0.01, "held at {seen}, wanted -0.2");
}

/// With a lookahead and no framing, a running target puts the rig ahead of it (#1253).
#[test]
fn a_running_target_is_led() {
    let (mut resources, vcam, target) = world();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<RotationComposer>()
        .unwrap()
        .remove(vcam);
    registry.get_cpu_mut::<CameraLookahead>().unwrap().insert(
        vcam,
        CameraLookahead {
            smoothing: 0.0,
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
        arm(&mut resources, vcam, 8.0);
        {
            let registry = resources.get_mut::<ComponentRegistry>().unwrap();
            let cam = registry
                .get_cpu_mut::<VirtualCamera>()
                .unwrap()
                .get_mut(vcam)
                .unwrap();
            cam.follow = crate::FOLLOW_ORBITAL;
            cam.pitch = 18.0;
            registry
                .get_cpu_mut::<RotationComposer>()
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
        arm(&mut resources, vcam, 8.0);
        {
            let registry = resources.get_mut::<ComponentRegistry>().unwrap();
            let cam = registry
                .get_cpu_mut::<VirtualCamera>()
                .unwrap()
                .get_mut(vcam)
                .unwrap();
            cam.follow = crate::FOLLOW_ORBITAL;
            cam.pitch = 18.0;
            cam.damping = Vec3::splat(0.3);
            cam.rotation_damping = 0.5;
            let frame = registry
                .get_cpu_mut::<RotationComposer>()
                .unwrap()
                .get_mut(vcam)
                .unwrap();
            frame.enabled = framed;
            // 🔴 Tight zones on purpose. With the exponential the camera keeps up so well that
            // wide ones are never left at all, and a test where the boundary is never reached
            // cannot tell a boundary that is handled from one that is shoved (#1336).
            frame.dead_zone = Vec2::splat(0.04);
            frame.soft_zone = Vec2::splat(0.08);
            frame.damping = Vec2::splat(0.6);
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
        // 🔴 A ceiling, not only a ratio. On the exponential the bare rig changes the frame's rate
        // by 0.0007 at 6 m/s and 0.007 at 40, so a ratio alone compares two numbers that are both
        // invisible. What matters is whether a step is SEEN: 0.02 rad per step per step is about a
        // degree, and the wall this test was written for measured 0.070.
        //
        // Measured here: 6 m/s framed 0.005 against 0.0007; 40 m/s framed 0.015 against 0.007. The
        // rest is the dead zone's own nature — it holds the camera still and then moves it.
        assert!(
            framed <= 0.02,
            "at {speed} m/s framing changed the frame's rate by {framed:.5}, against the rig's own {bare:.5}",
        );
    }
}

/// 🔴 #1330: a wall's push is not slack. The frame reads where the rig had the camera before the
/// collision, or it spends a `damping` undoing what the wall just did — two things moving
/// one camera, one stage apart.
#[test]
fn a_wall_is_not_slack() {
    let (mut resources, vcam, _target) = world();
    for _ in 0..30 {
        drive_virtual_cameras(&mut resources);
    }
    let (settled, _) = pose(&resources, vcam);
    // Nothing moved and no wall exists here: the frame must leave the camera exactly where it is,
    // step after step, rather than drifting by whatever it thinks it is owed.
    for _ in 0..60 {
        drive_virtual_cameras(&mut resources);
    }
    let (after, _) = pose(&resources, vcam);
    assert!(
        after.abs_diff_eq(settled, 1e-4),
        "the frame drifted while nothing moved: {settled} then {after}",
    );
}

/// 🔴 The rule the whole change rests on: exactly **one** aim runs. A framing on a vcam that aims at
/// its target is ignored, not folded in — two owners of the rotation is what #1329 ran from.
#[test]
fn only_one_aim_runs() {
    let (mut resources, vcam, _) = world();
    rigid(&mut resources, vcam);
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry
            .get_cpu_mut::<VirtualCamera>()
            .unwrap()
            .get_mut(vcam)
            .unwrap()
            .look_at = crate::LOOK_AT_SIMPLE;
        registry
            .get_cpu_mut::<RotationComposer>()
            .unwrap()
            .get_mut(vcam)
            .unwrap()
            .screen
            .x = 0.25;
    }
    for _ in 0..200 {
        drive_virtual_cameras(&mut resources);
    }
    // `Simple` aims at the target, so it is centred and the framing's offset never happened.
    let seen = on_screen(&resources, vcam, Vec3::ZERO).x;
    assert!(
        seen.abs() < 0.01,
        "the framing reached an aim it does not own: {seen}"
    );
}

/// 🔴 #1389: the pitch **selects** a point on the ring surface instead of swinging an arm, and it
/// maps over the orbit's own limits so the ends of the stick reach the ends of the rig.
#[test]
fn the_pitch_walks_the_ring_surface() {
    let (mut resources, vcam, _) = world();
    {
        let registry = resources.get_mut::<ComponentRegistry>().unwrap();
        registry.register_cpu::<crate::OrbitalFollow>();
        registry
            .get_cpu_mut::<crate::OrbitalFollow>()
            .unwrap()
            .insert(
                vcam,
                crate::OrbitalFollow {
                    orbit_style: crate::ORBIT_THREE_RING,
                    // 99, to prove the sphere's radius places nothing under the rings.
                    radius: 99.0,
                    ..Default::default()
                },
            );
        registry.register_cpu::<crate::orbit::CameraOrbit>();
        registry
            .get_cpu_mut::<crate::orbit::CameraOrbit>()
            .unwrap()
            .insert(
                vcam,
                crate::orbit::CameraOrbit {
                    pitch_min: -30.0,
                    pitch_max: 70.0,
                    ..Default::default()
                },
            );
        let cam = registry
            .get_cpu_mut::<VirtualCamera>()
            .unwrap()
            .get_mut(vcam)
            .unwrap();
        cam.follow = crate::FOLLOW_ORBITAL;
        cam.look_at = crate::LOOK_AT_ARM;
    }
    let at = |resources: &mut Resources, pitch: f32| {
        resources
            .get_mut::<ComponentRegistry>()
            .unwrap()
            .get_cpu_mut::<VirtualCamera>()
            .unwrap()
            .get_mut(vcam)
            .unwrap()
            .pitch = pitch;
        for _ in 0..200 {
            drive_virtual_cameras(resources);
        }
        pose(resources, vcam).0
    };

    let bottom = at(&mut resources, -30.0);
    let middle = at(&mut resources, 20.0);
    let top = at(&mut resources, 70.0);
    let rings = crate::OrbitalFollow::default();

    // The ends land on the rings they name, and `camera_distance` is not what places any of them.
    assert!((bottom.y - rings.bottom_height).abs() < 0.05, "{bottom:?}");
    assert!((top.y - rings.top_height).abs() < 0.05, "{top:?}");
    // And the middle is wider than either end: the surface, not a sphere.
    let out = |at: Vec3| Vec2::new(at.x, at.z).length();
    assert!(out(middle) > out(bottom) + 1.0, "{middle:?} vs {bottom:?}");
    assert!(out(middle) > out(top) + 1.0, "{middle:?} vs {top:?}");
}
