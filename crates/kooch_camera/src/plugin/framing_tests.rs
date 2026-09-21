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

#[test]
fn the_dead_zone_holds_the_rig() {
    let (mut resources, vcam, target) = world();
    for _ in 0..10 {
        drive_virtual_cameras(&mut resources);
    }
    let settled = pose(&resources, vcam);
    // 60° at 5 m over 16:9 is ~10 m wide: the default dead zone reaches ~0.5 m either side.
    place(&mut resources, target, Vec3::X * 0.3);
    for _ in 0..30 {
        drive_virtual_cameras(&mut resources);
    }
    assert_eq!(pose(&resources, vcam), settled);
}

#[test]
fn leaving_the_dead_zone_moves_it() {
    let (mut resources, vcam, target) = world();
    drive_virtual_cameras(&mut resources);
    place(&mut resources, target, Vec3::X * 4.0);
    for _ in 0..120 {
        drive_virtual_cameras(&mut resources);
    }
    let (position, _) = pose(&resources, vcam);
    // It stops once the target is back on the dead zone's edge, not centred.
    assert!(position.x > 3.0 && position.x < 4.0, "{position}");
}

#[test]
fn the_screen_offset_turns_it() {
    let (mut resources, vcam, _) = world();
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry
        .get_cpu_mut::<CameraFraming>()
        .unwrap()
        .get_mut(vcam)
        .unwrap()
        .screen
        .x = 0.25;
    drive_virtual_cameras(&mut resources);
    let (_, rotation) = pose(&resources, vcam);
    let forward = rotation * -Vec3::Z;
    // Framed right of centre, so the camera looks left of the target.
    assert!(forward.x < -0.1, "{forward}");
}
