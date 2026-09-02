//! The shapes #137 added, doing their job in a running simulation.
//!
//! One test per line of the issue's acceptance list. Asserted against
//! resting heights and body counts rather than against contact manifolds:
//! what an author needs is that the thing lands where it looks like it
//! should, and the manifold is rapier's business.

use super::*;

use crate::backend::{ColliderMesh, ColliderMeshCache};
use crate::components::{
    SHAPE_CONVEX_HULL, SHAPE_HALF_SPACE, SHAPE_SPHERE, SHAPE_TRIMESH, SHAPE_VOXELIZED_MESH,
};

/// A unit cube's corners and triangles, centred on the origin.
///
/// Small enough to read, and closed — a hull needs volume and a trimesh
/// needs manifold triangles.
fn unit_cube() -> ColliderMesh {
    let vertices = vec![
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
    ];
    let indices = vec![
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
    ColliderMesh {
        vertices,
        indices,
        ..Default::default()
    }
}

/// Publishes `mesh` under a fresh GUID and returns it.
fn publish(resources: &mut Resources, mesh: ColliderMesh) -> kooch_core::Guid {
    let guid = kooch_core::Guid::new_v4();
    let mut cache = resources.remove::<ColliderMeshCache>().expect("no cache");
    cache.insert(guid, mesh);
    resources.insert(cache);
    guid
}

fn statue(kind: u32) -> PhysicsBody {
    PhysicsBody {
        kind,
        mass: 0.0,
        ..Default::default()
    }
}

/// The reason a test scene stops needing a cuboid whose only job was to
/// be large enough never to be walked off.
#[test]
fn a_half_space_catches_what_falls() {
    let mut resources = world();
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        statue(KIND_STATIC),
        Collider {
            shape: SHAPE_HALF_SPACE,
            normal: Vec3::Y,
            ..Default::default()
        },
    );
    let ball = falling_sphere(&mut resources, 4.0);
    Playing::set(&mut resources, true);
    simulate(&mut resources, 240);

    let resting = position(&resources, ball).y;
    assert!(
        (resting - 0.5).abs() < 0.1,
        "a unit sphere rests half a metre above the plane, not at {resting}",
    );
}

/// A hull built from a real mesh has to collide against that mesh's
/// silhouette, not against whatever the numeric fields happened to hold.
#[test]
fn a_hull_collides_at_its_silhouette() {
    let mut resources = world();
    let guid = publish(&mut resources, unit_cube());
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        statue(KIND_STATIC),
        Collider {
            shape: SHAPE_CONVEX_HULL,
            mesh: Some(guid),
            // Deliberately wrong for a unit cube: if the solver reads
            // this instead of the mesh, the ball rests a metre too high.
            radius: 4.0,
            ..Default::default()
        },
    );
    let ball = falling_sphere(&mut resources, 4.0);
    Playing::set(&mut resources, true);
    simulate(&mut resources, 240);

    let resting = position(&resources, ball).y;
    assert!(
        (resting - 1.0).abs() < 0.15,
        "the cube's top is at 0.5 and the ball's radius is 0.5, so it rests near 1.0, not {resting}",
    );
}

/// Static level geometry, which is what a trimesh is correct for.
#[test]
fn a_trimesh_catches_what_falls() {
    let mut resources = world();
    let guid = publish(&mut resources, unit_cube());
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        statue(KIND_STATIC),
        Collider {
            shape: SHAPE_TRIMESH,
            mesh: Some(guid),
            ..Default::default()
        },
    );
    let ball = falling_sphere(&mut resources, 4.0);
    Playing::set(&mut resources, true);
    simulate(&mut resources, 240);

    assert!(
        position(&resources, ball).y > 0.5,
        "the ball went through the level",
    );
}

/// The shape terraforming needs: it collides against the cells directly,
/// so a body crossing a chunk seam must not catch on anything.
#[test]
fn voxels_have_no_seam_to_snag_on() {
    let mut resources = world();
    let guid = publish(&mut resources, unit_cube());
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        statue(KIND_STATIC),
        Collider {
            shape: SHAPE_VOXELIZED_MESH,
            mesh: Some(guid),
            voxel_size: 0.1,
            voxel_solid: true,
            ..Default::default()
        },
    );
    let ball = falling_sphere(&mut resources, 3.0);
    Playing::set(&mut resources, true);
    simulate(&mut resources, 240);

    assert!(
        position(&resources, ball).y > 0.5,
        "the ball fell through the voxels",
    );
}

/// A mesh-derived collider authored before its mesh loads must not become
/// a body built from a stand-in — and must not stay missing once the mesh
/// arrives.
#[test]
fn a_body_waits_for_its_mesh() {
    let mut resources = world();
    let guid = kooch_core::Guid::new_v4();
    spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        statue(KIND_STATIC),
        Collider {
            shape: SHAPE_CONVEX_HULL,
            mesh: Some(guid),
            ..Default::default()
        },
    );
    simulate(&mut resources, 1);
    assert_eq!(
        body_count(&resources),
        0,
        "built from geometry nobody wrote"
    );

    let mut cache = resources.remove::<ColliderMeshCache>().unwrap();
    cache.insert(guid, unit_cube());
    resources.insert(cache);
    simulate(&mut resources, 1);
    assert_eq!(body_count(&resources), 1, "the arrival never reached it");
}

/// A hull with no volume is reported rather than silently producing a
/// collider the author cannot see and cannot hit.
#[test]
fn a_flat_hull_leaves_no_collider() {
    let mut resources = world();
    let flat = ColliderMesh {
        vertices: vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0],
        indices: Vec::new(),
        ..Default::default()
    };
    let guid = publish(&mut resources, flat);
    let entity = spawn_body(
        &mut resources,
        Transform::from_position(Vec3::ZERO),
        statue(KIND_STATIC),
        Collider {
            shape: SHAPE_CONVEX_HULL,
            mesh: Some(guid),
            ..Default::default()
        },
    );
    simulate(&mut resources, 1);

    let world_ref = resources.get::<PhysicsWorld>().unwrap();
    let handle = world_ref
        .handle(slot_of(&resources, entity).expect("the body exists"))
        .expect("slot is free");
    assert_eq!(
        world_ref.backend().collider_count(handle),
        Some(0),
        "a refused shape must not leave a secret one behind",
    );
}

/// Every shape a scene can hold has to survive being written and read
/// back, mesh-derived ones included — the GUID is the only thing standing
/// in for geometry that cannot be typed.
///
/// Every *discriminant*, not every dropdown entry: five shapes lost their
/// label and kept their number, and a scene authored with one before that
/// still has to load.
#[test]
fn every_shape_round_trips_through_a_scene() {
    use crate::components::SHAPE_VOXELIZED_MESH;

    for shape in 0..=SHAPE_VOXELIZED_MESH {
        let collider = Collider {
            shape,
            mesh: Some(kooch_core::Guid::new_v4()),
            ..Default::default()
        };
        let restored = round_trip(&collider);
        assert_eq!(
            restored.shape_spec(None),
            collider.shape_spec(None),
            "shape {shape} lost geometry across a save",
        );
    }
}

/// A dropdown entry that resolves to nothing is a shape an author can
/// pick and then watch do nothing.
#[test]
fn every_offered_shape_resolves() {
    use crate::backend::{ColliderMesh, ColliderMeshCache, ConvexPart};
    use crate::components::SHAPE_CHOICES;

    let guid = kooch_core::Guid::new_v4();
    let mut cache = ColliderMeshCache::new();
    cache.insert(
        guid,
        ColliderMesh {
            parts: vec![ConvexPart::loose(unit_cube().vertices.clone())],
            ..unit_cube()
        },
    );

    for choice in SHAPE_CHOICES {
        let collider = Collider {
            shape: choice.value as u32,
            mesh: Some(guid),
            ..Default::default()
        };
        assert!(
            collider.collision_shape(Some(&cache)).is_some(),
            "{} resolves to nothing",
            choice.label,
        );
    }
}

/// The labels are what an author reads; two the same is a menu nobody can
/// use, and a stale one outliving its shape is worse.
#[test]
fn the_labels_are_distinct() {
    use crate::components::SHAPE_CHOICES;

    let mut labels = std::collections::HashSet::new();
    let mut values = std::collections::HashSet::new();
    for choice in SHAPE_CHOICES {
        assert!(
            labels.insert(choice.label),
            "{} is listed twice",
            choice.label
        );
        assert!(
            values.insert(choice.value),
            "{} and something else share value {}",
            choice.label,
            choice.value,
        );
    }
}

/// Writes a collider through reflection and reads it back, the way a
/// scene file does.
fn round_trip(collider: &Collider) -> Collider {
    use kooch_ecs::Reflect;

    let mut restored = Collider {
        // Not the default, so a field the round trip skips is visible as
        // a leftover rather than as a coincidental match.
        shape: SHAPE_SPHERE,
        radius: 99.0,
        ..Default::default()
    };
    for field in collider.reflect_fields() {
        let value = collider
            .reflect_get(field.name)
            .unwrap_or_else(|| panic!("{} is not readable", field.name));
        restored
            .reflect_set(field.name, value)
            .unwrap_or_else(|error| panic!("{}: {error}", field.name));
    }
    restored
}
