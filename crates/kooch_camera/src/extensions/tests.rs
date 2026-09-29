use super::*;

use glam::Quat;
use kooch_ecs::allocator::EntityAllocator;

use crate::frame::CameraFrame;
use crate::framing::Lens;
use crate::rig::RigMemory;
use crate::target::GroupPose;

const DT: f32 = 1.0 / 60.0;

fn lens() -> Lens {
    Lens::new(90.0, 1.0)
}

/// A camera 4 m back looking down −Z at a target on the origin, with `component` on its entity.
fn step<T: Component>(
    component: T,
    target: Vec3,
    up: Vec3,
    turned: Quat,
    run: fn(&mut RigStep),
) -> CameraFrame {
    let mut allocator = EntityAllocator::new();
    let entity = allocator.spawn();
    let mut registry = ComponentRegistry::new();
    registry.register_cpu::<T>();
    registry
        .get_cpu_mut::<T>()
        .unwrap()
        .insert(entity, component);
    registry.register_cpu::<crate::VirtualCamera>();

    let mut resources = kooch_core::resource::Resources::new();
    let carried = RigMemory::default();
    let mut memory = RigMemory::default();
    let vcam = crate::VirtualCamera::default();
    let eye = Vec3::Z * 4.0;
    let mut rig_step = RigStep {
        frame: CameraFrame::new(eye, eye, turned, target, lens()),
        entity,
        vcam: &vcam,
        target: GroupPose {
            position: target,
            rotation: Quat::IDENTITY,
            heaviest: entity,
        },
        up,
        reference: Vec3::Z,
        dt: DT,
        resources: &mut resources,
        registry: &registry,
        carried: &carried,
        memory: &mut memory,
    };
    run(&mut rig_step);
    rig_step.frame
}

/// Where `target` lands on screen for a frame's pose.
fn on_screen(frame: &CameraFrame, target: Vec3) -> Vec2 {
    let to = target - frame.position;
    let (right, above, forward) = frame.axes();
    let span = lens().span(to.dot(forward));
    Vec2::new(to.dot(right) / span.x, to.dot(above) / span.y)
}

/// 🔴 The offset is read in the **camera's** own frame, not the world's. Turned a quarter turn so
/// the two answers cannot coincide: a camera left unrotated tests nothing here.
#[test]
fn an_offset_moves_it_in_its_own_frame() {
    let offset = CameraOffset {
        offset: Vec3::new(1.0, 0.0, 0.0),
        ..Default::default()
    };
    // Looking down −X: the camera's own right is −Z.
    let turned = Quat::from_rotation_y(90.0_f32.to_radians());
    let frame = step(offset, Vec3::ZERO, Vec3::Y, turned, after_aim);
    let moved = frame.position - Vec3::Z * 4.0;
    assert!(
        moved.abs_diff_eq(Vec3::NEG_Z, 1e-4),
        "it moved along world X, not its own right: {moved:?}",
    );
    // The body's own answer is untouched, so a later stage still knows where it wanted to be.
    assert!(
        frame.free.abs_diff_eq(Vec3::Z * 4.0, 1e-4),
        "{:?}",
        frame.free
    );
}

/// A stage it did not name does nothing at all.
#[test]
fn a_stage_it_did_not_name_is_quiet() {
    let offset = CameraOffset {
        offset: Vec3::X,
        apply_after: 4,
        ..Default::default()
    };
    let frame = step(offset, Vec3::ZERO, Vec3::Y, Quat::IDENTITY, after_body);
    assert!(
        frame.position.abs_diff_eq(Vec3::Z * 4.0, 1e-4),
        "{:?}",
        frame.position
    );
}

/// 🔴 The whole of `preserve_composition`: moving the camera drags the target across the screen, and
/// this turns back so it lands where it did.
///
/// The target is held **off centre** on purpose. Centred, "put it back where it was" and "aim
/// straight at it" are the same answer, and the test passes with the offset thrown away.
#[test]
fn preserving_the_composition_turns_back() {
    let target = Vec3::X * 1.5;
    let eye = Vec3::Z * 4.0;
    let plain = CameraOffset {
        offset: Vec3::X * 2.0,
        ..Default::default()
    };
    let was = on_screen(
        &CameraFrame::new(eye, eye, Quat::IDENTITY, target, lens()),
        target,
    );
    assert!(was.x > 0.1, "the target should start off centre: {was:?}");

    let dragged = on_screen(
        &step(plain, target, Vec3::Y, Quat::IDENTITY, after_aim),
        target,
    );
    assert!(
        (dragged.x - was.x).abs() > 0.1,
        "the offset did not move it: {dragged:?}",
    );

    let preserved = CameraOffset {
        preserve_composition: true,
        ..plain
    };
    let held = on_screen(
        &step(preserved, target, Vec3::Y, Quat::IDENTITY, after_aim),
        target,
    );
    assert!(
        (held.x - was.x).abs() < 0.01,
        "it did not put the target back where it was: {held:?}, was {was:?}",
    );
}

/// A pan is about the vcam's up, so on the side of a planet it turns along the local horizon.
#[test]
fn a_pan_turns_about_the_vcam_up() {
    let recomposer = CameraRecomposer {
        pan: 30.0,
        ..Default::default()
    };
    let frame = step(recomposer, Vec3::ZERO, Vec3::Y, Quat::IDENTITY, after_aim);
    let right = frame.rotation * Vec3::X;
    assert!(
        right.dot(Vec3::Y).abs() < 1e-4,
        "the horizon rolled: {right:?}"
    );
    let forward = frame.rotation * Vec3::NEG_Z;
    assert!(forward.x < -0.4, "it did not pan: {forward:?}");
}

/// And a tilt is about the camera's own right, positive looking up.
#[test]
fn a_tilt_looks_up() {
    let recomposer = CameraRecomposer {
        tilt: 20.0,
        ..Default::default()
    };
    let frame = step(recomposer, Vec3::ZERO, Vec3::Y, Quat::IDENTITY, after_aim);
    let forward = frame.rotation * Vec3::NEG_Z;
    assert!(forward.y > 0.3, "it did not look up: {forward:?}");
}

/// Zeroes cost nothing and change nothing: a recomposer left alone is not a rotation of zero applied
/// every frame.
#[test]
fn a_quiet_recomposer_is_quiet() {
    let frame = step(
        CameraRecomposer::default(),
        Vec3::ZERO,
        Vec3::Y,
        Quat::IDENTITY,
        after_aim,
    );
    assert_eq!(frame.rotation, Quat::IDENTITY);
}
