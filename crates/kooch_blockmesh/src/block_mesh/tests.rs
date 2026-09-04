use glam::Vec3;

use crate::BlockMesh;

fn unit_cube() -> BlockMesh {
    BlockMesh::cuboid(Vec3::splat(0.5))
}

#[test]
fn a_cuboid_has_six_faces() {
    assert_eq!(unit_cube().face_count(), 6);
}

#[test]
fn a_cuboid_shares_eight_corners() {
    // Twenty-four face-corners over eight positions: the whole reason
    // this type exists instead of editing the render mesh.
    let cube = unit_cube();
    assert_eq!(cube.positions().len(), 8);
    assert_eq!(cube.faces().map(<[u32]>::len).sum::<usize>(), 24);
}

#[test]
fn every_face_names_four_corners() {
    for face in unit_cube().faces() {
        assert_eq!(face.len(), 4);
    }
}

#[test]
fn cuboid_normals_point_outward() {
    let cube = unit_cube();
    for index in 0..cube.face_count() {
        let face = cube.face(index).unwrap();
        let centre: Vec3 = face
            .iter()
            .map(|corner| cube.positions()[*corner as usize])
            .sum::<Vec3>()
            / face.len() as f32;
        let normal = cube.face_normal(index).unwrap();
        assert!(
            centre.dot(normal) > 0.0,
            "face {index} winds inward: centre {centre}, normal {normal}"
        );
    }
}

#[test]
fn a_cuboid_takes_its_extent() {
    let cube = BlockMesh::cuboid(Vec3::new(2.0, 3.0, 4.0));
    for position in cube.positions() {
        assert_eq!(position.abs(), Vec3::new(2.0, 3.0, 4.0));
    }
}

#[test]
fn triangles_fan_every_face() {
    // Six quads, two triangles each.
    assert_eq!(unit_cube().triangles().len(), 12);
}

#[test]
fn triangles_keep_shared_corners() {
    // Welded, because this feeds the collider.
    let cube = unit_cube();
    let highest = cube.triangles().iter().flatten().copied().max().unwrap();
    assert_eq!(highest as usize, cube.positions().len() - 1);
}

#[test]
fn an_empty_mesh_has_no_faces() {
    let empty = BlockMesh::default();
    assert_eq!(empty.face_count(), 0);
    assert!(empty.face(0).is_none());
    assert!(empty.triangles().is_empty());
}

#[test]
fn a_missing_corner_is_refused() {
    let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
    assert!(BlockMesh::from_faces(positions, &[vec![0, 1, 3]]).is_none());
}

#[test]
fn a_two_corner_face_is_refused() {
    let positions = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
    assert!(BlockMesh::from_faces(positions, &[vec![0, 1]]).is_none());
}

#[test]
fn from_faces_matches_cuboid() {
    let cube = unit_cube();
    let faces: Vec<Vec<u32>> = cube.faces().map(<[u32]>::to_vec).collect();
    let rebuilt = BlockMesh::from_faces(cube.positions().to_vec(), &faces).unwrap();
    assert_eq!(rebuilt, cube);
}

#[test]
fn ron_round_trips() {
    // The serialised surface is the part that cannot change later.
    let cube = unit_cube();
    let text = ron::to_string(&cube).unwrap();
    assert_eq!(ron::from_str::<BlockMesh>(&text).unwrap(), cube);
}

#[test]
fn ron_names_three_fields() {
    let text = ron::to_string(&unit_cube()).unwrap();
    for field in ["positions", "face_corners", "face_starts"] {
        assert!(text.contains(field), "{field} missing from {text}");
    }
}
