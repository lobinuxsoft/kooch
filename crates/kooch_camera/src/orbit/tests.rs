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
            follow: crate::FOLLOW_ORBITAL,
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

fn orbit_of(resources: &Resources, vcam: Entity) -> CameraOrbit {
    *resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<CameraOrbit>()
        .unwrap()
        .get(vcam)
        .unwrap()
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
        orbit.auto_recentre = false;
        orbit.recentre_wait = 0.1;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..600 {
        orbit_cameras(&mut resources);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!((yaw - 90.0).abs() < 1e-4, "yaw was {yaw}");
}

/// 🔴 The switch owns the automatic mode and nothing else: off, the tailing stops the same step —
/// and a return asked for by hand keeps going, because with the switch off that is the only kind
/// there is (#1350).
#[test]
fn the_switch_owns_only_the_auto_mode() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.auto_recentre = true;
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 0.5;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..12 {
        orbit_cameras(&mut resources);
    }
    let (caught, _) = angles(&resources, vcam);
    assert!(
        caught < 89.0,
        "the automatic return never started: {caught}"
    );

    orbit(&mut resources, vcam, |orbit| orbit.auto_recentre = false);
    orbit_cameras(&mut resources);
    let (stopped, _) = angles(&resources, vcam);
    for _ in 0..600 {
        orbit_cameras(&mut resources);
    }
    let (after, _) = angles(&resources, vcam);
    assert!(
        (after - stopped).abs() < 1e-4,
        "the tailing outlived its switch: {stopped} then {after}"
    );
    let wait = orbit_of(&resources, vcam).recentre_wait;
    assert!((wait - 0.1).abs() < 1e-6, "the wait was lost: {wait}");

    // And the button still works, which is the whole point of the switch being off.
    orbit(&mut resources, vcam, |orbit| orbit.recentre_now = true);
    orbit_cameras(&mut resources);
    orbit(&mut resources, vcam, |orbit| orbit.recentre_now = false);
    for _ in 0..120 {
        orbit_cameras(&mut resources);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!(
        yaw.abs() < 1.0,
        "the press did nothing with the switch off: {yaw}"
    );
}

#[test]
fn a_waited_yaw_returns_behind() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.auto_recentre = true;
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
        orbit.auto_recentre = true;
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

/// 🔴 The point of the button: it returns the camera with the automatic switch **off**, which is
/// how most third-person games ship it — R3 recentres, nothing recentres on its own (#1348).
#[test]
fn a_press_returns_it_with_auto_off() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.auto_recentre = false;
        orbit.recentre_wait = 60.0;
        orbit.recentre_time = 0.25;
        orbit.recentre_now = true;
    });
    set_yaw(&mut resources, vcam, 90.0);
    orbit_cameras(&mut resources);
    orbit(&mut resources, vcam, |orbit| orbit.recentre_now = false);
    for _ in 0..60 {
        orbit_cameras(&mut resources);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!(yaw.abs() < 1.0, "the press never returned it: {yaw}");
}

/// 🔴 A manual return **ends where it arrives, and a hold does not re-arm it**. The angle cannot say
/// so — parked behind the target, another return changes nothing to look at — so the state is what
/// the test reads.
#[test]
fn a_held_press_asks_once() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre_time = 0.1;
        orbit.recentre_now = true;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..60 {
        orbit_cameras(&mut resources);
        // Still held down, every step.
        orbit(&mut resources, vcam, |orbit| orbit.recentre_now = true);
    }
    let (yaw, _) = angles(&resources, vcam);
    assert!(yaw.abs() < 1.0, "the return never arrived: {yaw}");
    assert!(
        !orbit_of(&resources, vcam).returning_now,
        "the hold asked for another return once the first arrived"
    );

    // 🔴 And the button, still held, must not tail the target the way the automatic mode does. This
    // is the only place the press edge is observable: parked behind a still target, a return asked
    // for again changes nothing to look at.
    let parked = yaw;
    let mut facing = 0.0;
    for _ in 0..120 {
        facing += 2.0;
        turn_target(&mut resources, facing);
        orbit(&mut resources, vcam, |orbit| orbit.recentre_now = true);
        orbit_cameras(&mut resources);
    }
    let (after, _) = angles(&resources, vcam);
    assert!(
        (after - parked).abs() < 1e-3,
        "the held button tailed the target: {parked} then {after}"
    );
}

/// A look beats the button, as it beats the timer.
#[test]
fn a_look_cancels_a_pressed_return() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre_time = 2.0;
        orbit.recentre_now = true;
    });
    set_yaw(&mut resources, vcam, 90.0);
    for _ in 0..15 {
        orbit_cameras(&mut resources);
    }
    let (caught, _) = angles(&resources, vcam);
    assert!(caught < 89.0, "the return never started: {caught}");

    orbit(&mut resources, vcam, |orbit| {
        orbit.recentre_now = false;
        orbit.look = Vec2::X;
        orbit.speed = Vec2::ZERO;
    });
    orbit_cameras(&mut resources);
    let (held, _) = angles(&resources, vcam);
    orbit(&mut resources, vcam, |orbit| orbit.look = Vec2::ZERO);
    for _ in 0..60 {
        orbit_cameras(&mut resources);
    }
    let (after, _) = angles(&resources, vcam);
    assert!(
        (after - held).abs() < 1e-4,
        "the look did not cancel it: {held} then {after}"
    );
}

/// 🔴 The automatic mode is a **mode**, not one return: it holds the camera behind a target that
/// keeps turning, for as long as nobody looks (#1350). Cinemachine reads the same — its axis rests
/// at `Center`, and `Center` is behind the target.
#[test]
fn auto_holds_the_camera_behind() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.auto_recentre = true;
        orbit.recentre_wait = 0.1;
        orbit.recentre_time = 0.1;
    });

    // Settle behind a still target first, then start steering.
    for _ in 0..120 {
        orbit_cameras(&mut resources);
    }
    let mut facing = 0.0;
    for _ in 0..240 {
        facing += 1.0;
        turn_target(&mut resources, facing);
        orbit_cameras(&mut resources);
    }
    // Behind a target facing `facing` is the yaw that followed it round.
    let (yaw, _) = angles(&resources, vcam);
    let gap = (yaw - facing).rem_euclid(360.0);
    let gap = gap.min(360.0 - gap);
    assert!(
        gap < 10.0,
        "the camera stopped tailing the target: {yaw} vs {facing}"
    );
}

/// And a look arms the next one: the window is per idle period, not per scene.
#[test]
fn a_look_arms_the_next_return() {
    let (mut resources, vcam) = world();
    orbit(&mut resources, vcam, |orbit| {
        orbit.auto_recentre = true;
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
        orbit.auto_recentre = true;
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
