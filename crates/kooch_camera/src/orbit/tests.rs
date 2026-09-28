//! Turning a vcam from a rate, and the yaw's way back (#1258).

use super::*;
use crate::target::CameraTarget;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::transform::Transform;

const STEP: f32 = 1.0 / 60.0;

/// A third-person vcam with an orbit, and a target facing `-Z` at the origin.
fn world() -> (Resources, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<CameraTarget>();
    registry.register_cpu_reflected::<CameraOrbit>();
    let (vcam, target) = (allocator.spawn(), allocator.spawn());
    let transforms = registry.get_cpu_mut::<Transform>().unwrap();
    transforms.insert(vcam, Transform::default());
    transforms.insert(target, Transform::default());
    registry.get_cpu_mut::<VirtualCamera>().unwrap().insert(
        vcam,
        VirtualCamera {
            follow: crate::FOLLOW_THIRD_PERSON,
            ..Default::default()
        },
    );
    registry
        .get_cpu_mut::<CameraTarget>()
        .unwrap()
        .insert(target, CameraTarget::default());
    registry
        .get_cpu_mut::<CameraOrbit>()
        .unwrap()
        .insert(vcam, CameraOrbit::default());
    resources.insert(allocator);
    resources.insert(registry);
    (resources, vcam)
}

fn orbit(resources: &mut Resources, vcam: Entity, change: impl Fn(&mut CameraOrbit)) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    let orbit = registry
        .get_cpu_mut::<CameraOrbit>()
        .unwrap()
        .get_mut(vcam)
        .unwrap();
    change(orbit);
}

fn angles(resources: &Resources, vcam: Entity) -> (f32, f32) {
    let vcam = resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<VirtualCamera>()
        .unwrap()
        .get(vcam)
        .unwrap();
    (vcam.yaw, vcam.pitch)
}

#[test]
fn a_look_turns_the_yaw() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| orbit.look = Vec2::X);
    orbit_cameras(&mut resources);
    let (yaw, _) = angles(&resources, vcam);
    assert!((yaw + 180.0 * STEP).abs() < 1e-4, "yaw was {yaw}");
}

#[test]
fn an_invert_turns_it_back() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.look = Vec2::X;
        orbit.invert_yaw = true;
    });
    orbit_cameras(&mut resources);
    let (yaw, _) = angles(&resources, vcam);
    assert!((yaw - 180.0 * STEP).abs() < 1e-4, "yaw was {yaw}");
}

#[test]
fn the_speed_is_per_axis() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.look = Vec2::ONE;
        orbit.speed = Vec2::new(90.0, 360.0);
    });
    orbit_cameras(&mut resources);
    let (yaw, pitch) = angles(&resources, vcam);
    assert!((yaw + 90.0 * STEP).abs() < 1e-4, "yaw was {yaw}");
    // From the default 20, down by the vertical rate.
    assert!(
        (pitch - (20.0 - 360.0 * STEP)).abs() < 1e-4,
        "pitch was {pitch}"
    );
}

#[test]
fn the_pitch_stops_at_its_limit() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.look = -Vec2::Y;
        orbit.speed = Vec2::splat(10_000.0);
    });
    orbit_cameras(&mut resources);
    let (_, pitch) = angles(&resources, vcam);
    assert!((pitch - 70.0).abs() < 1e-4, "pitch was {pitch}");
}

/// Off is off, however long nobody looks — and it is the default.
#[test]
fn the_switch_off_never_recentres() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre = false;
        orbit.recentre_wait = 0.1;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..600 {
        orbit_cameras(&mut resources);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!((yaw - 90.0).abs() < 1e-4, "yaw was {yaw}");
}

/// 🔴 The switch, not a zero wait: turning it off at runtime must not cost the authored timing
/// (#1333's shape). Flipped mid-return, the camera stops where it is.
#[test]
fn the_switch_stops_a_live_return() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre = true;
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 2.0;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..30 {
        orbit_cameras(&mut resources);
    }
    let (caught, _) = angles(&resources, vcam);
    assert!(caught < 89.0, "the return never started: {caught}");

    orbit(&mut resources, vcam, |orbit| orbit.recentre = false);
    for _ in 0..120 {
        orbit_cameras(&mut resources);
    }
    let (held, _) = angles(&resources, vcam);
    assert!((held - caught).abs() < 1e-4, "it kept returning: {held}");
    let wait = resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<CameraOrbit>()
        .unwrap()
        .get(vcam)
        .unwrap()
        .recentre_wait;
    assert!((wait - 0.1).abs() < 1e-6, "the wait was lost: {wait}");
}

#[test]
fn a_waited_yaw_returns_behind() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre = true;
        orbit.recentre_wait = 0.5;
        orbit.recentre_time = 0.25;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..120 {
        orbit_cameras(&mut resources);
    }
    // The target faces `-Z`, so behind it is `+Z` — the reference the arm seeds, which is yaw zero.
    let (yaw, _) = angles(&resources, vcam);
    assert!(yaw.abs() < 1.0, "yaw was {yaw}");
}

#[test]
fn a_look_cancels_the_return() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre = true;
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 0.5;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..30 {
        orbit_cameras(&mut resources);
    }
    let (turning, _) = angles(&resources, vcam);
    assert!(turning < 89.0, "the return never started: {turning}");

    // A hand back on the stick: the wait starts over, so the next step turns rather than returns.
    orbit(&mut resources, vcam, |orbit| {
        orbit.look = Vec2::X;
        orbit.speed = Vec2::ZERO;
    });
    orbit_cameras(&mut resources);
    let (held, _) = angles(&resources, vcam);
    orbit_cameras(&mut resources);
    let (still, _) = angles(&resources, vcam);
    assert!(
        (still - held).abs() < 1e-4,
        "the return resumed under a held stick: {held} then {still}"
    );
}

/// 🔴 The defect from the smoke test, and the one its first fix missed: the return ends while the
/// target is still turning. An asymptotic ease closes a fraction of the gap per step, so against a
/// character that keeps steering the gap settles at a constant lag and an arrival test is never
/// reached — the window has to be what ends it (#1345).
#[test]
fn a_turning_target_ends_the_return() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre = true;
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 0.25;
    });
    set_yaw(&mut resources, vcam, 90.0);

    // The character keeps steering: three degrees a step, which is 180 a second.
    let mut facing = 0.0;
    let mut steer = |resources: &mut Resources| {
        facing += 3.0;
        turn_target(resources, facing);
        orbit_cameras(resources);
    };
    for _ in 0..120 {
        steer(&mut resources);
    }
    let (before, _) = angles(&resources, vcam);
    for _ in 0..120 {
        steer(&mut resources);
    }
    let (after, _) = angles(&resources, vcam);
    assert!(
        (after - before).abs() < 1e-3,
        "the camera never stopped chasing the target's facing: {before} then {after}"
    );
}

/// And a look arms the next one: the window is per idle period, not per scene.
#[test]
fn a_look_arms_the_next_return() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre = true;
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 0.25;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..120 {
        orbit_cameras(&mut resources);
    }

    orbit(&mut resources, vcam, |orbit| orbit.look = Vec2::X);
    orbit_cameras(&mut resources);
    orbit(&mut resources, vcam, |orbit| orbit.look = Vec2::ZERO);
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..120 {
        orbit_cameras(&mut resources);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!(yaw.abs() < 1.0, "the second return never ran: {yaw}");
}

/// 🔴 A yaw past a full turn returns the way it came, not the long way round: `nearest` is why the
/// return crosses 360 instead of unwinding 350 degrees.
#[test]
fn a_return_takes_the_short_way() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre = true;
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 0.5;
    });
    set_yaw(&mut resources, vcam, 350.0);
    for _ in 0..12 {
        orbit_cameras(&mut resources);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!(yaw > 350.0, "the return unwound the long way: {yaw}");
}

/// The yaw behind a target is measured around the local up, not around world up.
#[test]
fn behind_is_measured_on_the_up() {
    // Standing on a wall: up is `+X`, and a target looking along `-Z` puts behind it at `+Z`.
    let reference = Vec3::Z;
    let turn = behind(Vec3::NEG_Z, Vec3::X, reference).unwrap();
    assert!(turn.abs() < 1e-3, "turn was {turn}");
    // Turned a quarter on that same horizon.
    let turn = behind(Vec3::NEG_Y, Vec3::X, reference).unwrap();
    assert!((turn.abs() - 90.0).abs() < 1e-3, "turn was {turn}");
}

#[test]
fn the_nearest_angle_wraps() {
    assert!((nearest(350.0, 10.0) - 370.0).abs() < 1e-4);
    assert!((nearest(10.0, 350.0) + 10.0).abs() < 1e-4);
    assert!((nearest(0.0, 0.0)).abs() < 1e-4);
}

/// Turns the target on the spot, which is what moves "behind" it.
fn turn_target(resources: &mut Resources, degrees: f32) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    let targets: Vec<Entity> = registry
        .get_cpu::<CameraTarget>()
        .unwrap()
        .iter()
        .map(|(&entity, _)| entity)
        .collect();
    let transforms = registry.get_cpu_mut::<Transform>().unwrap();
    for entity in targets {
        let transform = transforms.get_mut(entity).unwrap();
        transform.rotation = glam::Quat::from_rotation_y(degrees.to_radians());
    }
}

fn set_yaw(resources: &mut Resources, vcam: Entity, yaw: f32) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<VirtualCamera>()
        .unwrap()
        .get_mut(vcam)
        .unwrap()
        .yaw = yaw;
}
