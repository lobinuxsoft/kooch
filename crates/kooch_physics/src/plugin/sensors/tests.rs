use super::*;
use glam::Mat4;
use kooch_ecs::allocator::EntityAllocator;

use crate::components::SHAPE_TRIMESH;

/// A world with one sensor of `shape` at the origin and one body at `at`.
fn world(shape: u32, collider: Collider, at: Vec3) -> (ComponentRegistry, Entity, Entity) {
    let mut allocator = EntityAllocator::new();
    let (sensor, body) = (allocator.spawn(), allocator.spawn());
    let mut registry = ComponentRegistry::new();
    registry.register_cpu_reflected::<Collider>();
    registry.register_cpu_reflected::<GlobalTransform>();
    registry
        .get_cpu_mut::<Collider>()
        .expect("registered")
        .insert(
            sensor,
            Collider {
                shape,
                sensor: true,
                ..collider
            },
        );
    let transforms = registry
        .get_cpu_mut::<GlobalTransform>()
        .expect("registered");
    transforms.insert(
        sensor,
        GlobalTransform {
            matrix: Mat4::IDENTITY,
        },
    );
    transforms.insert(
        body,
        GlobalTransform {
            matrix: Mat4::from_translation(at),
        },
    );
    (registry, sensor, body)
}

#[test]
fn a_sphere_measures_from_its_surface() {
    let collider = Collider {
        radius: 5.0,
        ..Default::default()
    };
    let (registry, sensor, body) = world(SHAPE_SPHERE, collider, Vec3::new(3.0, 0.0, 0.0));
    assert_eq!(depth_of(&registry, sensor, body), 2.0);
}

/// 🔴 The nearest face, not the furthest: a long corridor entered from the end is a metre inside
/// after a metre, however long it is.
#[test]
fn a_box_measures_its_nearest_face() {
    let collider = Collider {
        half_extents: Vec3::new(20.0, 2.0, 2.0),
        ..Default::default()
    };
    let (registry, sensor, body) = world(SHAPE_CUBOID, collider, Vec3::new(0.0, 1.5, 0.0));
    assert_eq!(depth_of(&registry, sensor, body), 0.5);
}

/// Outside reads negative, which is what keeps a weight from claiming a body that has left before
/// the departure lands.
#[test]
fn outside_reads_negative() {
    let collider = Collider {
        radius: 1.0,
        ..Default::default()
    };
    let (registry, sensor, body) = world(SHAPE_SPHERE, collider, Vec3::new(4.0, 0.0, 0.0));
    assert!(depth_of(&registry, sensor, body) < 0.0);
}

/// A shape with no cheap answer is all of it, from the moment the solver says the body arrived.
#[test]
fn an_unmeasured_shape_is_all_of_it() {
    let (registry, sensor, body) = world(SHAPE_TRIMESH, Collider::default(), Vec3::ZERO);
    assert_eq!(depth_of(&registry, sensor, body), f32::INFINITY);
}

/// The region's own transform counts: a scaled sensor is a bigger region, not a bigger number.
#[test]
fn a_scaled_region_scales_with_it() {
    let collider = Collider {
        radius: 1.0,
        ..Default::default()
    };
    let (mut registry, sensor, body) = world(SHAPE_SPHERE, collider, Vec3::new(3.0, 0.0, 0.0));
    registry
        .get_cpu_mut::<GlobalTransform>()
        .expect("registered")
        .insert(
            sensor,
            GlobalTransform {
                matrix: Mat4::from_scale(Vec3::splat(4.0)),
            },
        );
    // Three metres out, in a sphere scaled to four: a quarter of the radius in local space.
    assert_eq!(depth_of(&registry, sensor, body), 0.25);
}
