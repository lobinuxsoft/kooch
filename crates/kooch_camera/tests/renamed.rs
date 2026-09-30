//! A scene authored before #1367 still loads: the components it names and the fields it sets were
//! renamed to Cinemachine's, and every one of them answers to what it used to be called.
//!
//! Run with:
//!   cargo test -p kooch_camera --test renamed

use kooch_camera::{Deoccluder, RotationComposer, VirtualCamera};
use kooch_ecs::Reflect;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::reflect::ReflectValue;

fn registry() -> ComponentRegistry {
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<VirtualCamera>();
    registry.register_cpu_reflected::<RotationComposer>();
    registry.register_cpu_reflected::<Deoccluder>();
    registry
}

#[test]
fn the_old_type_names_still_load() {
    let registry = registry();
    for (was, now) in [
        (
            "kooch_camera::framing::CameraFraming",
            std::any::type_name::<RotationComposer>(),
        ),
        (
            "kooch_camera::occlusion::CameraCollision",
            std::any::type_name::<Deoccluder>(),
        ),
    ] {
        assert_eq!(
            registry.type_id_by_name(was),
            registry.type_id_by_name(now),
            "{was} no longer resolves, so it would vanish from every scene that names it",
        );
    }
}

#[test]
fn the_old_field_names_still_load() {
    let mut vcam = VirtualCamera::default();
    for (was, value) in [
        ("distance", 9.0_f32),
        ("shoulder", 0.0),
        ("arm_rise", 1.5),
        ("side", 0.25),
    ] {
        let set = match was {
            "shoulder" => vcam.reflect_set(was, ReflectValue::Vec3(glam::Vec3::splat(0.5))),
            _ => vcam.reflect_set(was, ReflectValue::F32(value)),
        };
        assert!(set.is_ok(), "{was} was dropped: {set:?}");
    }
    // 🔴 These moved to the body components in #1391, so the alias lands them in the hidden fields a
    // migration empties — the value survives the load, which is what the alias is for.
    assert_eq!(vcam.was_distance, 9.0);
    assert_eq!(vcam.was_shoulder, glam::Vec3::splat(0.5));
    assert_eq!(vcam.was_arm_length, 1.5);
    assert_eq!(vcam.was_side, 0.25);
}
