//! A camera that outranks the others while its condition holds (#1352).

use super::*;
use crate::when::CameraWhen;
use kooch_ecs::allocator::EntityAllocator;

/// Two vcams following one target, the second authored lower than the first.
fn world() -> (Resources, Entity, Entity) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Transform>();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<CameraTarget>();
    registry.register_cpu_reflected::<CameraWhen>();
    let (orbital, shoulder, target) = (allocator.spawn(), allocator.spawn(), allocator.spawn());
    let transforms = registry.get_cpu_mut::<Transform>().expect("registered");
    for entity in [orbital, shoulder, target] {
        transforms.insert(entity, Transform::default());
    }
    let vcams = registry.get_cpu_mut::<VirtualCamera>().expect("registered");
    for (entity, priority) in [(orbital, 5), (shoulder, 1)] {
        vcams.insert(
            entity,
            VirtualCamera {
                priority,
                follow: crate::FOLLOW_ORBITAL,
                ..Default::default()
            },
        );
    }
    registry
        .get_cpu_mut::<CameraTarget>()
        .expect("registered")
        .insert(target, CameraTarget::default());
    resources.insert(allocator);
    resources.insert(registry);
    resources.insert(crate::rig::CameraRig::standard());
    (resources, orbital, shoulder)
}

fn hold(resources: &mut Resources, vcam: Entity, asked: bool) {
    let registry = resources
        .get_mut::<ComponentRegistry>()
        .expect("registered");
    let whens = registry.get_cpu_mut::<CameraWhen>().expect("registered");
    if whens.get(vcam).is_none() {
        whens.insert(
            vcam,
            CameraWhen {
                boost: 10,
                ..Default::default()
            },
        );
    }
    whens.get_mut(vcam).expect("just inserted").asked = asked;
}

fn winner(resources: &Resources) -> Entity {
    let (plan, _) = plan_vcam_poses(resources);
    elect(&plan).expect("two vcams follow the target").0
}

fn authored(resources: &Resources, vcam: Entity) -> i32 {
    resources
        .get::<ComponentRegistry>()
        .expect("registered")
        .get_cpu::<VirtualCamera>()
        .expect("registered")
        .get(vcam)
        .expect("spawned")
        .priority
}

/// The lower-authored camera wins while it is asked for, and hands back when it is not.
#[test]
fn a_held_camera_outranks_the_others() {
    let (mut resources, orbital, shoulder) = world();
    assert_eq!(winner(&resources), orbital);

    hold(&mut resources, shoulder, true);
    crate::when::step_camera_whens(&mut resources);
    assert_eq!(winner(&resources), shoulder);

    hold(&mut resources, shoulder, false);
    crate::when::step_camera_whens(&mut resources);
    assert_eq!(winner(&resources), orbital);
}

/// 🔴 The authored number is never written. A component that overwrote `priority` would be a second
/// owner of it, and letting go would not give the authored value back.
#[test]
fn the_authored_priority_survives() {
    let (mut resources, orbital, shoulder) = world();
    hold(&mut resources, shoulder, true);
    crate::when::step_camera_whens(&mut resources);
    let _ = winner(&resources);
    assert_eq!(authored(&resources, shoulder), 1);
    assert_eq!(authored(&resources, orbital), 5);
}

/// A boost too small to beat the authored gap loses, which is what makes the number worth authoring.
#[test]
fn too_small_a_boost_still_loses() {
    let (mut resources, orbital, shoulder) = world();
    hold(&mut resources, shoulder, true);
    {
        let registry = resources
            .get_mut::<ComponentRegistry>()
            .expect("registered");
        let when = registry
            .get_cpu_mut::<CameraWhen>()
            .expect("registered")
            .get_mut(shoulder)
            .expect("held");
        when.boost = 2;
    }
    crate::when::step_camera_whens(&mut resources);
    assert_eq!(winner(&resources), orbital);
}
