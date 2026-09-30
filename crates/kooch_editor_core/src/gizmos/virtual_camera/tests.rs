//! 🔴 #1379: a composer in the Aim cancels the shoulder's effect on screen, so the only way to see
//! that the shoulder is working is to draw it. These check the chain reaches the screen.

use super::*;
use crate::gizmos::harness::{draw, draw_with};
use glam::Mat4;
use kooch_camera::target::CameraTarget;
use kooch_ecs::component::ComponentRegistry;

fn shoulder() -> VirtualCamera {
    VirtualCamera {
        ..Default::default()
    }
}

/// The body that carries the shoulder's numbers (#1391).
fn shoulder_body() -> kooch_camera::ThirdPersonFollow {
    kooch_camera::ThirdPersonFollow {
        shoulder_offset: Vec3::new(0.6, -0.4, 0.0),
        vertical_arm_length: 1.2,
        camera_side: 1.0,
        camera_distance: 2.5,
    }
}

/// A world with a target on the origin. `planned` is whether the rig has run — 🔴 the editor's own
/// world **never has**: it adds `CameraComponentsPlugin` and not `CameraPlugin`, so `RigMemory` is
/// not even inserted there, and a test that supplied one built the world the code wanted rather
/// than the one that exists (#1387).
fn world(vcam: VirtualCamera, entity: Entity, planned: bool, shouldered: bool) -> Resources {
    let mut resources = Resources::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<VirtualCamera>();
    registry.register_cpu::<CameraTarget>();
    registry.register_cpu::<GlobalTransform>();
    registry.register_cpu::<kooch_camera::ThirdPersonFollow>();
    if shouldered {
        registry
            .get_cpu_mut::<kooch_camera::ThirdPersonFollow>()
            .unwrap()
            .insert(entity, shoulder_body());
    }
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

    if planned {
        let mut memory = RigMemory::default();
        memory.horizons.set(entity, Vec3::Y, Vec3::Z);
        resources.insert(memory);
    }
    resources
}

/// How many of `drawn` are the chain: segments touching a pivot the plain gizmo never draws.
fn chain(drawn: &[(Vec3, Vec3)], plain: &[(Vec3, Vec3)]) -> usize {
    drawn.len() - plain.len()
}

/// 🔴 The whole point, in the world the editor actually has: **no `RigMemory`**, because the rig
/// has not run and in the editor's process never will. The chain is what a shoulder is tuned by, and
/// tuning happens stopped.
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
        &world(vcam, entity, false, true),
    );
    // Three segments for the chain, three strokes at each of its three pivots, and the orbit the arm
    // swings along — which also moved here, because its length is the body's now (#1391).
    assert!(
        chain(&drawn, &plain) >= 3 + 9,
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
        ..Default::default()
    };
    let at = Mat4::from_translation(Vec3::Z * 2.5);
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);
    let drawn = draw_with(
        &VirtualCameraVisualizer,
        &vcam,
        at,
        entity,
        &world(vcam, entity, false, false),
    );
    assert_eq!(chain(&drawn, &plain), 0);
}

/// 🔴 And a rig that has **run** draws the same chain from what it used, rather than from the
/// first-step answer — while playing, the memory is the only honest source and the fallback must not
/// quietly take over.
#[test]
fn a_planned_rig_draws_from_its_memory() {
    let entity = Entity::new(1, 0);
    let vcam = shoulder();
    let at = Mat4::from_translation(Vec3::new(0.6, 0.8, 2.5));
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);
    let drawn = draw_with(
        &VirtualCameraVisualizer,
        &vcam,
        at,
        entity,
        &world(vcam, entity, true, true),
    );
    assert!(chain(&drawn, &plain) >= 3 + 9);
}

/// A body that is not the spring arm has no chain to draw.
#[test]
fn another_body_draws_no_chain() {
    let entity = Entity::new(1, 0);
    let vcam = VirtualCamera {
        ..Default::default()
    };
    let at = Mat4::IDENTITY;
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);
    let drawn = draw_with(
        &VirtualCameraVisualizer,
        &vcam,
        at,
        entity,
        &world(vcam, entity, false, false),
    );
    assert_eq!(chain(&drawn, &plain), 0);
}

/// 🔴 #1389: the ring surface is the thing being tuned, and three numbers in a list are not a shape.
#[test]
fn the_rings_draw_their_surface() {
    let rings = kooch_camera::ORBIT_THREE_RING;
    let entity = Entity::new(1, 0);
    let vcam = VirtualCamera {
        ..Default::default()
    };
    let at = Mat4::from_translation(Vec3::Z * 4.0);
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);

    let mut resources = world(vcam, entity, false, false);
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu::<kooch_camera::OrbitalFollow>();
    registry
        .get_cpu_mut::<kooch_camera::OrbitalFollow>()
        .unwrap()
        .insert(
            entity,
            kooch_camera::OrbitalFollow {
                orbit_style: rings,
                radius: 4.0,
                ..Default::default()
            },
        );

    let drawn = draw_with(&VirtualCameraVisualizer, &vcam, at, entity, &resources);
    assert!(
        chain(&drawn, &plain) > 24,
        "the surface and its three circles are not there: {} extra",
        chain(&drawn, &plain),
    );
}

/// And a sphere draws none of it: the rings are not the surface it rides.
#[test]
fn a_sphere_draws_no_rings() {
    let rings = kooch_camera::ORBIT_SPHERE;
    let entity = Entity::new(1, 0);
    let vcam = VirtualCamera {
        ..Default::default()
    };
    let at = Mat4::from_translation(Vec3::Z * 4.0);
    let plain = draw(&VirtualCameraVisualizer, &vcam, at);

    let mut resources = world(vcam, entity, false, false);
    let registry = resources.get_mut::<ComponentRegistry>().unwrap();
    registry.register_cpu::<kooch_camera::OrbitalFollow>();
    registry
        .get_cpu_mut::<kooch_camera::OrbitalFollow>()
        .unwrap()
        .insert(
            entity,
            kooch_camera::OrbitalFollow {
                orbit_style: rings,
                radius: 4.0,
                ..Default::default()
            },
        );

    let drawn = draw_with(&VirtualCameraVisualizer, &vcam, at, entity, &resources);
    assert_eq!(chain(&drawn, &plain), 0);
}
