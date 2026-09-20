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

/// 🔴 Declared, never guessed. Picking the highest-priority camera handed the rig to whatever
/// overlay outranked the base, and a scene where nobody said which camera the rig is for has no
/// answer to give — moving one at random is the bug, not the fallback.
#[test]
fn a_nameless_scene_drives_nothing() {
    let (resources, cameras) = world(&[(0, false, None), (1, true, None)]);
    assert_eq!(rendering_camera(&resources, cameras[1]), None);
}

/// What a scene says is the whole answer, whatever the priorities imply.
#[test]
fn a_brain_takes_the_rig() {
    let (resources, cameras) = world(&[(0, false, None), (5, false, Some(true))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[1]));
    // And the same the other way round: a brain on the lower-priority camera still wins.
    let (resources, cameras) = world(&[(9, false, None), (0, false, Some(true))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[1]));
}

/// Several brains is an ordering question again, and priority is the order.
#[test]
fn the_top_brain_takes_the_rig() {
    let (resources, cameras) = world(&[(0, false, Some(true)), (5, false, Some(true))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[1]));
}

/// A brain that is switched off is not one, and switching the only one off parks the rig — which is
/// what an author who unticked it asked for.
#[test]
fn a_disabled_brain_drives_nothing() {
    let (resources, cameras) = world(&[(0, false, None), (5, true, Some(false))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), None);
}

/// An overlay CAN be driven — it just has to say so. A weapon camera with its own rig is the case.
#[test]
fn a_brain_on_an_overlay_drives() {
    let (resources, cameras) = world(&[(0, false, None), (1, true, Some(true))]);
    assert_eq!(rendering_camera(&resources, cameras[0]), Some(cameras[1]));
}
