//! Who the frame's cameras are, and in what order (#1221).

use glam::Mat4;
use kooch_core::resource::Resources;
use kooch_ecs::allocator::EntityAllocator;
use kooch_ecs::archetype_registry::ArchetypeRegistry;
use kooch_ecs::commands::Commands;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::query::AccessTracker;

use super::CameraStack;

/// A scene holding the cameras it was given, each already placed.
fn scene(cameras: &[PerspectiveCamera]) -> Resources {
    let mut resources = Resources::new();
    resources.insert(EntityAllocator::new());
    resources.insert(ComponentRegistry::new());
    resources.insert(ArchetypeRegistry::new());
    resources.insert(AccessTracker::new());
    let mut commands = Commands::new();
    for camera in cameras {
        commands
            .spawn(&mut resources)
            .insert(*camera)
            .insert(GlobalTransform {
                matrix: Mat4::IDENTITY,
            });
    }
    commands.apply(&mut resources);
    resources
}

/// A camera at `priority`, an overlay or not, with a lens to tell it apart by.
fn camera(priority: i32, overlay: bool) -> PerspectiveCamera {
    PerspectiveCamera {
        priority,
        overlay,
        fov: 30.0 + priority as f32,
        ..Default::default()
    }
}

/// The lens each camera of the stack arrived with, in degrees.
fn lenses(stack: &CameraStack) -> Vec<f32> {
    stack
        .overlays
        .iter()
        .map(|(_, view)| view.fov_y_rad.to_degrees().round())
        .collect()
}

/// 🔴 An overlay is not a candidate for the base, however high it sits: picking by priority alone
/// would hand the image to the overlay and leave the base out of the frame.
#[test]
fn an_overlay_is_never_base() {
    let resources = scene(&[camera(0, false), camera(10, true)]);
    let stack = CameraStack::read::<()>(&resources);
    let base = stack.base.expect("the camera that is not an overlay");
    assert_eq!(base.fov_y_rad, 30.0_f32.to_radians());
    assert_eq!(lenses(&stack), vec![40.0]);
}

#[test]
fn overlays_compose_by_priority() {
    let resources = scene(&[
        camera(0, false),
        camera(5, true),
        camera(1, true),
        camera(9, true),
    ]);
    let stack = CameraStack::read::<()>(&resources);
    assert_eq!(lenses(&stack), vec![31.0, 35.0, 39.0]);
}

/// 🔴 Nothing under it means nothing to keep where it drew nothing, so an overlay alone would read as
/// the whole image — the one case where composing is worse than not.
#[test]
fn a_baseless_overlay_is_dropped() {
    let resources = scene(&[camera(3, true), camera(1, true)]);
    let stack = CameraStack::read::<()>(&resources);
    assert!(stack.base.is_none());
    assert!(stack.overlays.is_empty());
}

#[test]
fn an_inactive_camera_is_out() {
    let off = PerspectiveCamera {
        active: false,
        ..camera(7, true)
    };
    let resources = scene(&[camera(0, false), off]);
    let stack = CameraStack::read::<()>(&resources);
    assert!(stack.base.is_some());
    assert!(stack.overlays.is_empty());
}
