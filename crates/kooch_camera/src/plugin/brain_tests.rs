//! Which camera the rig drives (#1221): the one that says so, and — where nothing says — the base.

use super::*;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::component::ComponentRegistry;

/// A world of cameras, each `(priority, overlay, brain)`, in the order they were spawned.
fn world(cameras: &[(i32, bool, Option<bool>)]) -> (Resources, Vec<Entity>) {
    let mut resources = Resources::new();
    let mut allocator = EntityAllocator::new();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<PerspectiveCamera>();
    registry.register_cpu_reflected::<CameraBrain>();
    let mut entities = Vec::new();
    for (priority, overlay, brain) in cameras {
        let entity = allocator.spawn();
        registry
            .get_cpu_mut::<PerspectiveCamera>()
            .expect("registered")
            .insert(
                entity,
                PerspectiveCamera {
                    priority: *priority,
                    overlay: *overlay,
                    ..Default::default()
                },
            );
        if let Some(enabled) = brain {
            registry
                .get_cpu_mut::<CameraBrain>()
                .expect("registered")
                .insert(entity, CameraBrain { enabled: *enabled });
        }
        entities.push(entity);
    }
    resources.insert(allocator);
    resources.insert(registry);
    (resources, entities)
}

/// 🔴 The bug camera stacking introduced: "highest priority" was the renderer's own rule while a
/// frame was one camera. An overlay that outranks the base took the rig, and the base stopped
/// following its target — the game view went black around a character that still moved.
#[test]
fn an_overlay_never_takes_the_rig() {
    let (resources, cameras) = world(&[(0, false, None), (1, true, None)]);
    assert_eq!(
        rendering_camera(&resources, cameras[1]),
        Some(cameras[0]),
        "the rig went to the overlay",
    );
}

/// What a scene says beats what the priorities imply.
#[test]
fn a_brain_takes_the_rig() {
    let (resources, cameras) = world(&[(0, false, None), (5, false, Some(true))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[1]));
    // And the same the other way round: a brain on the lower-priority camera still wins.
    let (resources, cameras) = world(&[(9, false, None), (0, false, Some(true))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[1]));
}

/// A brain that is switched off is not one: the camera it sits on is not a candidate, and a scene
/// whose every brain is off falls back rather than freezing the rig.
#[test]
fn a_disabled_brain_falls_back() {
    let (resources, cameras) = world(&[(0, false, None), (5, true, Some(false))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[0]));
}

/// An overlay CAN be driven — it just has to say so. A weapon camera with its own rig is the case.
#[test]
fn a_brain_on_an_overlay_drives() {
    let (resources, cameras) = world(&[(0, false, None), (1, true, Some(true))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[1]));
}
