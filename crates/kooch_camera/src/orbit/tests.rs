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

#[test]
fn a_still_look_never_recentres() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| orbit.look = Vec2::ZERO);
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..600 {
        orbit_cameras(&mut resources);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!((yaw - 90.0).abs() < 1e-4, "yaw was {yaw}");
}

#[test]
fn a_waited_yaw_returns_behind() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
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
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 0.1;
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

/// 🔴 A yaw past a full turn returns the way it came, not the long way round: `nearest` is why the
/// return crosses 360 instead of unwinding 350 degrees.
#[test]
fn a_return_takes_the_short_way() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
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

fn set_yaw(resources: &mut Resources, vcam: Entity, yaw: f32) {
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<VirtualCamera>()
        .unwrap()
        .get_mut(vcam)
        .unwrap()
        .yaw = yaw;
}
