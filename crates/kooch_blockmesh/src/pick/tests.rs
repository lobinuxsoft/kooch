use glam::Vec3;

use super::face_at;
use crate::BlockMesh;

/// A unit cube centred on the origin: faces at ±0.5 on each axis.
fn cube() -> BlockMesh {
    BlockMesh::cuboid(Vec3::splat(0.5))
}

/// The face whose outward normal points along `axis`.
fn facing(mesh: &BlockMesh, axis: Vec3) -> u32 {
    (0..mesh.face_count())
        .find(|face| mesh.face_normal(*face).unwrap().dot(axis) > 0.99)
        .expect("a cube faces every axis") as u32
}

#[test]
fn a_ray_hits_the_face_it_points_at() {
    let mesh = cube();
    for axis in [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z] {
        // Stand off along the axis and look back at the centre.
        let hit = face_at(&mesh, axis * 3.0, -axis).expect("the cube is in the way");
        assert_eq!(hit.face, facing(&mesh, axis), "looking down {axis}");
    }
}

#[test]
fn the_distance_reaches_the_surface() {
    // From 3 units out, the near face of a half-extent-0.5 cube is 2.5.
    let hit = face_at(&cube(), Vec3::Z * 3.0, -Vec3::Z).unwrap();
    assert!((hit.distance - 2.5).abs() < 1e-4, "{}", hit.distance);
}

#[test]
fn the_near_face_wins() {
    // The ray crosses two faces; the one it reaches first is the answer.
    let mesh = cube();
    let hit = face_at(&mesh, Vec3::Z * 3.0, -Vec3::Z).unwrap();
    assert_eq!(hit.face, facing(&mesh, Vec3::Z));
    assert_ne!(hit.face, facing(&mesh, -Vec3::Z));
}

#[test]
fn a_ray_that_misses_answers_none() {
    assert!(face_at(&cube(), Vec3::new(5.0, 5.0, 3.0), -Vec3::Z).is_none());
}

#[test]
fn nothing_behind_the_eye_is_hit() {
    // Standing in front and looking away.
    assert!(face_at(&cube(), Vec3::Z * 3.0, Vec3::Z).is_none());
}

#[test]
fn a_ray_from_inside_hits_a_wall() {
    // 🔴 Two-sided on purpose: a block being edited is routinely seen
    // from inside, and culling backfaces makes the wall in front of you
    // unselectable exactly then.
    let mesh = cube();
    let hit = face_at(&mesh, Vec3::ZERO, Vec3::Z).expect("the far wall is still a face");
    assert_eq!(hit.face, facing(&mesh, Vec3::Z));
    assert!((hit.distance - 0.5).abs() < 1e-4);
}

#[test]
fn a_grazing_ray_still_lands() {
    // Straight at a corner of the +Z face. An epsilon too tight drops
    // it, and a face becomes unclickable along its own edges.
    let mesh = cube();
    let hit = face_at(&mesh, Vec3::new(0.5, 0.5, 3.0), -Vec3::Z);
    assert_eq!(hit.map(|h| h.face), Some(facing(&mesh, Vec3::Z)));
}

#[test]
fn an_unnormalised_ray_scales_its_answer() {
    // `distance` is in units of `direction`, so a doubled direction
    // halves it. What a caller comparing two `t` on one ray wants.
    let mesh = cube();
    let slow = face_at(&mesh, Vec3::Z * 3.0, -Vec3::Z).unwrap();
    let fast = face_at(&mesh, Vec3::Z * 3.0, -Vec3::Z * 2.0).unwrap();
    assert!((slow.distance - fast.distance * 2.0).abs() < 1e-4);
}

#[test]
fn an_empty_mesh_is_never_hit() {
    assert!(face_at(&BlockMesh::default(), Vec3::Z * 3.0, -Vec3::Z).is_none());
}
