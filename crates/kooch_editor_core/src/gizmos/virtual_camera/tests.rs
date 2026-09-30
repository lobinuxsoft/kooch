//! 🔴 #1379: a composer in the Aim cancels the shoulder's effect on screen, so the only way to see
//! that the shoulder is working is to draw it. These check the chain reaches the screen.

use super::*;
use crate::gizmos::harness::{draw, draw_with};
use glam::Mat4;
use kooch_camera::target::CameraTarget;
use kooch_ecs::component::ComponentRegistry;

fn shoulder() -> VirtualCamera {
    VirtualCamera {
        follow: kooch_camera::FOLLOW_SHOULDER,
        camera_distance: 2.5,
        shoulder_offset: Vec3::new(0.6, -0.4, 0.0),
        vertical_arm_length: 1.2,
        ..Default::default()
    }
}

/// A world with a target on the origin, and the rig's memory of what it used.
fn world(vcam: VirtualCamera, entity: Entity, planned: bool) -> Resources {
    let mut resources = Resources::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<VirtualCamera>();
    registry.register_cpu::<CameraTarget>();
    registry.register_cpu::<GlobalTransform>();
    registry
        .get_cpu_mut::<VirtualCamera>()
        .unwrap()
        .insert(entity, vcam);
    let target = Entity::new(9, 0);
    registry
        .get_cpu_mut::<CameraTarget>()
        .unwrap()
        .insert(target, CameraTarget::default());
    registry.get_cpu_mut::<GlobalTransform>().unwrap().insert(
        target,
        GlobalTransform {
            matrix: Mat4::IDENTITY,
        },
    );
    resources.insert(registry);

    let mut memory = RigMemory::default();
    if planned {
        memory.horizons.set(entity, Vec3::Y, Vec3::Z);
    }
    resources.insert(memory);
    resources
}

/// How many of `drawn` are the chain: segments touching a pivot the plain gizmo never draws.
fn chain(drawn: &[(Vec3, Vec3)], plain: &[(Vec3, Vec3)]) -> usize {
    drawn.len() - plain.len()
}

/// The whole point: a shoulder that the aim hides on screen is still on screen here.
#[test]
fn a_shoulder_draws_its_chain() {
    let entity = Entity::new(1, 0);
    let vcam = shoulder();
    let at = Mat4::from_translation(Vec3::new(0.6, 0.8, 2.5));
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);
    let drawn = draw_with(
        &VirtualCameraVisualizer,
        &vcam,
        at,
        entity,
        &world(vcam, entity, true),
    );
    // Three segments for the chain and three strokes at each of its three pivots.
    assert_eq!(
        chain(&drawn, &plain),
        3 + 9,
        "{} extra",
        chain(&drawn, &plain)
    );
}

/// 🔴 And nothing authored draws nothing extra. A chain of stubs on every orbital vcam would be
/// noise on top of the arm it already draws.
#[test]
fn a_plain_rig_draws_no_chain() {
    let entity = Entity::new(1, 0);
    let vcam = VirtualCamera {
        follow: FOLLOW_ORBITAL,
        camera_distance: 2.5,
        ..Default::default()
    };
    let at = Mat4::from_translation(Vec3::Z * 2.5);
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);
    let drawn = draw_with(
        &VirtualCameraVisualizer,
        &vcam,
        at,
        entity,
        &world(vcam, entity, true),
    );
    assert_eq!(chain(&drawn, &plain), 0);
}

/// A vcam the rig has not planned has no up to draw the chain on, and guessing one would draw a rig
/// that is not the one running.
#[test]
fn an_unplanned_rig_is_silent() {
    let entity = Entity::new(1, 0);
    let vcam = shoulder();
    let at = Mat4::from_translation(Vec3::Z * 2.5);
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);
    let drawn = draw_with(
        &VirtualCameraVisualizer,
        &vcam,
        at,
        entity,
        &world(vcam, entity, false),
    );
    assert_eq!(chain(&drawn, &plain), 0);
}

/// A body that is not the spring arm has no chain to draw.
#[test]
fn another_body_draws_no_chain() {
    let entity = Entity::new(1, 0);
    let vcam = VirtualCamera {
        follow: kooch_camera::FOLLOW_SIMPLE,
        shoulder_offset: Vec3::X,
        ..Default::default()
    };
    let at = Mat4::IDENTITY;
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);
    let drawn = draw_with(
        &VirtualCameraVisualizer,
        &vcam,
        at,
        entity,
        &world(vcam, entity, true),
    );
    assert_eq!(chain(&drawn, &plain), 0);
}
