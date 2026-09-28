//! The Aim stage (#1331): the camera looks at its target from where the **body** left it, not from
//! where the body was heading.

use super::*;
use kooch_ecs::allocator::EntityAllocator;

/// A heavily damped `Simple` vcam 5 m behind a target at the origin, with a rigid aim so the test
/// reads the stage and not its easing.
fn world() -> (Resources, Entity, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<CameraTarget>();
    let (vcam, target) = (allocator.spawn(), allocator.spawn());
    let at = |position| Transform {
        position,
        ..Default::default()
    };
    let transforms = registry.get_cpu_mut::<Transform>().unwrap();
    transforms.insert(vcam, at(Vec3::Z * 5.0));
    transforms.insert(target, at(Vec3::ZERO));
    registry.get_cpu_mut::<VirtualCamera>().unwrap().insert(
        vcam,
        VirtualCamera {
            follow: crate::FOLLOW_SIMPLE,
            offset: Vec3::Z * 5.0,
            // Slow enough that one step leaves the camera nowhere near where it is going.
            damping_value: Vec3::splat(2.0),
            rotation_damping_value: 0.0,
            ..Default::default()
        },
    );
    registry
        .get_cpu_mut::<CameraTarget>()
        .unwrap()
        .insert(target, CameraTarget::default());
    resources.insert(allocator);
    resources.insert(registry);
    resources.insert(CameraRig::standard());
    (resources, vcam, target)
}

fn pose(resources: &Resources, entity: Entity) -> Transform {
    *resources
        .get::<ComponentRegistry>()
        .unwrap()
        .get_cpu::<Transform>()
        .unwrap()
        .get(entity)
        .unwrap()
}

fn place(resources: &mut Resources, entity: Entity, position: Vec3) {
    resources
        .get_mut::<ComponentRegistry>()
        .unwrap()
        .get_cpu_mut::<Transform>()
        .unwrap()
        .get_mut(entity)
        .unwrap()
        .position = position;
}

/// 🔴 The aim used to be computed from the follow mode's answer — where the body is going — while
/// the camera stood where the damping had reached. A camera chasing a target that jumped sideways
/// pointed at empty space for as long as the damping took.
#[test]
fn the_aim_follows_the_body() {
    let (mut resources, vcam, target) = world();
    place(&mut resources, target, Vec3::X * 10.0);
    drive_virtual_cameras(&mut resources);

    let camera = pose(&resources, vcam);
    // Damped hard: one step at 2 s leaves the camera barely off its mark, which is the whole point —
    // the two answers are metres apart.
    assert!(
        camera.position.x < 1.0,
        "the body arrived too fast to tell the answers apart: {}",
        camera.position.x,
    );

    let forward = camera.rotation * -Vec3::Z;
    let at_target = (Vec3::X * 10.0 - camera.position).normalize();
    assert!(
        forward.angle_between(at_target) < 1e-3,
        "the camera looked {:?} instead of {at_target:?}",
        forward,
    );
}
