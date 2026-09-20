use super::*;
use glam::Mat4;
use kooch_ecs::allocator::EntityAllocator;

use crate::backend::ColliderMeshCache;
use crate::components::{SHAPE_CONVEX_HULL, SHAPE_TRIMESH};

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
    assert_eq!(depth_of(&registry, None, sensor, body), 2.0);
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
    assert_eq!(depth_of(&registry, None, sensor, body), 0.5);
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
    assert!(depth_of(&registry, None, sensor, body) < 0.0);
}

/// A shape with no cheap answer is all of it, from the moment the solver says the body arrived.
#[test]
fn an_unmeasured_shape_is_all_of_it() {
    let (registry, sensor, body) = world(SHAPE_TRIMESH, Collider::default(), Vec3::ZERO);
    assert_eq!(depth_of(&registry, None, sensor, body), f32::INFINITY);
}

/// 🔴 Metres, not shape units. A blend distance is a distance in the world, and measuring in the
/// shape's own space made every scaled region blend that many times too fast — an author had to type
/// a tenth of what they meant, or a hundredth.
#[test]
fn a_scaled_region_measures_metres() {
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
    // A unit sphere scaled by four is four metres of radius; three metres out is one metre in.
    assert_eq!(depth_of(&registry, None, sensor, body), 1.0);
}

/// The shape's own centre, which is where the gizmo draws it: a region offset from its entity was
/// measured from the entity, so half of it read as outside.
#[test]
fn an_offset_shape_measures_from_itself() {
    let collider = Collider {
        radius: 1.0,
        center: Vec3::new(5.0, 0.0, 0.0),
        ..Default::default()
    };
    let (registry, sensor, body) = world(SHAPE_SPHERE, collider, Vec3::new(5.0, 0.0, 0.0));
    assert_eq!(depth_of(&registry, None, sensor, body), 1.0);
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

/// A unit cube's hull, with the faces that say it is one, under the asset a hull collider names.
fn cube_hull(guid: kooch_core::Guid) -> ColliderMeshCache {
    let points = vec![
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ];
    let faces = vec![
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [2, 3, 7],
        [2, 7, 6],
        [1, 2, 6],
        [1, 6, 5],
        [0, 4, 7],
        [0, 7, 3],
    ];
    let mut cache = ColliderMeshCache::new();
    cache.insert(
        guid,
        crate::backend::ColliderMesh {
            vertices: points.clone(),
            hull: crate::backend::ConvexPart { points, faces },
            ..Default::default()
        },
    );
    cache
}

/// 🔴 A block is never centred on its entity and is never a primitive, so the plane distance is the
/// only measurement that answers for one. Exact, and indifferent to where the mesh sits.
#[test]
fn a_hull_measures_its_nearest_face() {
    let guid = kooch_core::Guid::new_v4();
    let collider = Collider {
        shape: SHAPE_CONVEX_HULL,
        mesh: Some(guid),
        ..Default::default()
    };
    let (registry, sensor, body) = world(SHAPE_CONVEX_HULL, collider, Vec3::new(0.0, 0.5, 0.0));
    let meshes = cube_hull(guid);
    // Half a metre up inside a cube that reaches one metre: half a metre to the ceiling, and more
    // to every wall.
    let depth = depth_of(&registry, Some(&meshes), sensor, body);
    assert!((depth - 0.5).abs() < 1e-4, "it measured {depth}");
}

/// Winding is not something a generated mesh promises. A hull whose faces are wound the other way
/// would read as a plane everything is outside of, and the region would never fire.
#[test]
fn a_hull_ignores_its_winding() {
    let guid = kooch_core::Guid::new_v4();
    let collider = Collider {
        shape: SHAPE_CONVEX_HULL,
        mesh: Some(guid),
        ..Default::default()
    };
    let (registry, sensor, body) = world(SHAPE_CONVEX_HULL, collider, Vec3::ZERO);
    let mut meshes = cube_hull(guid);
    let mut flipped = crate::backend::ColliderMesh::default();
    if let Some(shape) = meshes.get(guid) {
        flipped.vertices = shape.vertices.clone();
        flipped.hull = crate::backend::ConvexPart {
            points: shape.hull.points.clone(),
            faces: shape
                .hull
                .faces
                .iter()
                .map(|face| [face[0], face[2], face[1]])
                .collect(),
        };
    }
    meshes.insert(guid, flipped);
    let depth = depth_of(&registry, Some(&meshes), sensor, body);
    assert!((depth - 1.0).abs() < 1e-4, "it measured {depth}");
}

/// 🔴 A region is every collider under it: an awkward space is covered with as many shapes as it
/// takes, and the deepest answers. One shape per region would make a corridor a chain of volumes
/// fighting each other by priority.
#[test]
fn a_region_is_all_its_colliders() {
    let collider = Collider {
        half_extents: Vec3::splat(1.0),
        ..Default::default()
    };
    let (mut registry, sensor, body) = world(SHAPE_CUBOID, collider, Vec3::new(6.0, 0.0, 0.0));
    // A second box of the region, three metres along, with the body deep inside it.
    let wing = Entity::new(9, 0);
    registry
        .get_cpu_mut::<Collider>()
        .expect("registered")
        .insert(
            wing,
            Collider {
                shape: SHAPE_CUBOID,
                half_extents: Vec3::splat(3.0),
                ..Default::default()
            },
        );
    registry
        .get_cpu_mut::<GlobalTransform>()
        .expect("registered")
        .insert(
            wing,
            GlobalTransform {
                matrix: Mat4::from_translation(Vec3::new(6.0, 0.0, 0.0)),
            },
        );
    registry.register_cpu_reflected::<kooch_ecs::hierarchy::Children>();
    registry
        .get_cpu_mut::<kooch_ecs::hierarchy::Children>()
        .expect("registered")
        .insert(
            sensor,
            kooch_ecs::hierarchy::Children {
                entities: vec![wing],
            },
        );

    // Outside the region's own box, three metres inside its wing.
    let depth = depth_of(&registry, None, sensor, body);
    assert!((depth - 3.0).abs() < 1e-4, "it measured {depth}");
}
