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

/// 🔴 The editor runs no solver, and the Game panel is where an author looks to see whether a
/// region works. Without this, a volume is dead everywhere except a built game.
#[test]
fn a_preview_fills_without_a_solver() {
    use kooch_core::resource::Resources;
    use kooch_ecs::post_process_volume::PostProcessVolume;
    use kooch_ecs::sensor_occupancy::SensorOccupancy;

    let collider = Collider {
        half_extents: Vec3::new(4.0, 4.0, 4.0),
        ..Default::default()
    };
    let (mut registry, region, body) = world(SHAPE_CUBOID, collider, Vec3::new(1.0, 0.0, 0.0));
    registry.register_cpu_reflected::<PostProcessVolume>();
    registry
        .get_cpu_mut::<PostProcessVolume>()
        .expect("registered")
        .insert(region, PostProcessVolume::default());
    // The body needs a collider of its own to be a candidate at all: a region catches colliders.
    registry
        .get_cpu_mut::<Collider>()
        .expect("registered")
        .insert(body, Collider::default());

    let mut resources = Resources::new();
    resources.insert(registry);
    resources.insert(SensorOccupancy::default());
    super::sensor_occupancy_preview_system(&mut resources);

    let inside = resources.get::<SensorOccupancy>().expect("still there");
    let depth = inside.depth_in(region).expect("the body is inside");
    assert!((depth - 3.0).abs() < 0.01, "it measured {depth}");
}

/// A region is not inside itself, and one region inside another says nothing about where the game
/// is: both would leave a volume permanently on.
#[test]
fn a_preview_ignores_other_regions() {
    use kooch_core::resource::Resources;
    use kooch_ecs::post_process_volume::PostProcessVolume;
    use kooch_ecs::sensor_occupancy::SensorOccupancy;

    let collider = Collider {
        half_extents: Vec3::splat(4.0),
        ..Default::default()
    };
    let (mut registry, region, other) = world(SHAPE_CUBOID, collider, Vec3::ZERO);
    registry.register_cpu_reflected::<PostProcessVolume>();
    registry
        .get_cpu_mut::<PostProcessVolume>()
        .expect("registered")
        .insert(region, PostProcessVolume::default());
    registry
        .get_cpu_mut::<Collider>()
        .expect("registered")
        .insert(
            other,
            Collider {
                sensor: true,
                ..Default::default()
            },
        );

    let mut resources = Resources::new();
    resources.insert(registry);
    resources.insert(SensorOccupancy::default());
    super::sensor_occupancy_preview_system(&mut resources);
    assert!(
        resources
            .get::<SensorOccupancy>()
            .expect("still there")
            .is_empty()
    );
}
