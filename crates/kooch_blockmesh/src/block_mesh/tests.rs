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

/// 🔴 Its own extension, never `ron`. `Material` claims `ron`, and two
/// loaders on one extension had the asset scan type a block as a
/// material — the inspector drew it with a base colour and nothing
/// could load it as what it is.
#[test]
fn a_block_does_not_claim_ron() {
    use kooch_core::asset_loader::AssetLoader;

    let extensions = crate::BlockMeshLoader.extensions();
    assert_eq!(extensions, &[crate::BLOCK_MESH_EXTENSION]);
    assert!(!extensions.contains(&"ron"), "ron belongs to Material");
}

/// The face whose outward normal points along `axis`.
fn facing(mesh: &BlockMesh, axis: Vec3) -> u32 {
    (0..mesh.face_count())
        .find(|face| mesh.face_normal(*face).unwrap().dot(axis) > 0.99)
        .expect("a cube faces every axis") as u32
}

#[test]
fn a_face_names_four_corners() {
    assert_eq!(unit_cube().corners_of(&[0]).len(), 4);
}

#[test]
fn two_faces_share_their_edge() {
    // Adjacent faces on a cube: 4 + 4 corners, two of them the same.
    let cube = unit_cube();
    let faces = [facing(&cube, Vec3::Z), facing(&cube, Vec3::X)];
    assert_eq!(cube.corners_of(&faces).len(), 6);
}

#[test]
fn every_face_covers_every_corner() {
    let cube = unit_cube();
    let all: Vec<u32> = (0..cube.face_count() as u32).collect();
    assert_eq!(cube.corners_of(&all).len(), 8);
}

#[test]
fn dragging_a_face_leaves_the_opposite_one() {
    // 🔴 The test the whole authoring mesh exists for. Moving +Z by one
    // metre must move its four corners and NOT the four behind them.
    let mut cube = unit_cube();
    let front = facing(&cube, Vec3::Z);
    let back = facing(&cube, -Vec3::Z);
    let before: Vec<Vec3> = cube
        .corners_of(&[back])
        .iter()
        .map(|corner| cube.positions()[*corner as usize])
        .collect();

    let moved = cube.corners_of(&[front]);
    cube.move_corners(&moved, Vec3::Z);

    let after: Vec<Vec3> = cube
        .corners_of(&[back])
        .iter()
        .map(|corner| cube.positions()[*corner as usize])
        .collect();
    assert_eq!(before, after, "the opposite face moved");
    assert!((cube.face_normal(front as usize).unwrap().z - 1.0).abs() < 1e-5);
}

#[test]
fn a_shared_corner_moves_once() {
    // Two adjacent faces selected together: the corners on their shared
    // edge must travel one metre, not two.
    let mut cube = unit_cube();
    let faces = [facing(&cube, Vec3::Z), facing(&cube, Vec3::X)];
    let corners = cube.corners_of(&faces);
    let before = cube.positions().to_vec();

    cube.move_corners(&corners, Vec3::Y);

    for corner in &corners {
        let moved = cube.positions()[*corner as usize] - before[*corner as usize];
        assert!((moved - Vec3::Y).length() < 1e-5, "moved {moved}");
    }
}

#[test]
fn the_centre_of_a_face_is_its_middle() {
    let cube = unit_cube();
    let front = facing(&cube, Vec3::Z);
    let centre = cube.centre_of(&[front]).unwrap();
    assert!(
        (centre - Vec3::new(0.0, 0.0, 0.5)).length() < 1e-5,
        "{centre}"
    );
}

#[test]
fn nothing_selected_has_no_centre() {
    assert!(unit_cube().centre_of(&[]).is_none());
}

#[test]
fn an_unknown_face_contributes_nothing() {
    assert!(unit_cube().corners_of(&[99]).is_empty());
}

#[test]
fn turning_a_face_keeps_its_centre() {
    // 🔴 About the selection's own centre, so the face TURNS. About the
    // mesh origin it would swing away — a translation nobody asked for.
    let mut cube = unit_cube();
    let front = facing(&cube, Vec3::Z);
    let corners = cube.corners_of(&[front]);
    let pivot = cube.centre_of(&[front]).unwrap();

    cube.turn_corners(&corners, pivot, glam::Quat::from_rotation_z(0.5));

    let after = cube.centre_of(&[front]).unwrap();
    assert!((after - pivot).length() < 1e-5, "the face moved: {after}");
}

#[test]
fn turning_leaves_the_other_corners() {
    let mut cube = unit_cube();
    let front = facing(&cube, Vec3::Z);
    let back = facing(&cube, -Vec3::Z);
    let before: Vec<Vec3> = cube
        .corners_of(&[back])
        .iter()
        .map(|c| cube.positions()[*c as usize])
        .collect();

    let corners = cube.corners_of(&[front]);
    let pivot = cube.centre_of(&[front]).unwrap();
    cube.turn_corners(&corners, pivot, glam::Quat::from_rotation_z(0.5));

    let after: Vec<Vec3> = cube
        .corners_of(&[back])
        .iter()
        .map(|c| cube.positions()[*c as usize])
        .collect();
    assert_eq!(before, after);
}

#[test]
fn scaling_a_face_keeps_its_centre() {
    let mut cube = unit_cube();
    let front = facing(&cube, Vec3::Z);
    let corners = cube.corners_of(&[front]);
    let pivot = cube.centre_of(&[front]).unwrap();

    cube.scale_corners(&corners, pivot, Vec3::splat(2.0));

    let after = cube.centre_of(&[front]).unwrap();
    assert!((after - pivot).length() < 1e-5);
}

#[test]
fn scaling_a_face_widens_it() {
    let mut cube = unit_cube();
    let front = facing(&cube, Vec3::Z);
    let corners = cube.corners_of(&[front]);
    let pivot = cube.centre_of(&[front]).unwrap();
    let before = cube.positions()[corners[0] as usize];

    cube.scale_corners(&corners, pivot, Vec3::new(2.0, 2.0, 1.0));

    let after = cube.positions()[corners[0] as usize];
    assert!((after - pivot).length() > (before - pivot).length());
}

#[test]
fn a_face_cannot_be_scaled_to_nothing() {
    // Collapsed onto the pivot, every later scale multiplies zero, and
    // the face can never be recovered by dragging back.
    let mut cube = unit_cube();
    let front = facing(&cube, Vec3::Z);
    let corners = cube.corners_of(&[front]);
    let pivot = cube.centre_of(&[front]).unwrap();

    cube.scale_corners(&corners, pivot, Vec3::ZERO);

    let corner = cube.positions()[corners[0] as usize];
    assert!((corner - pivot).length() > 0.0, "the face collapsed");
}

#[test]
fn positions_go_back_whole() {
    // What an undo puts back.
    let mut cube = unit_cube();
    let before = cube.positions().to_vec();
    let front = facing(&cube, Vec3::Z);
    let corners = cube.corners_of(&[front]);
    cube.move_corners(&corners, Vec3::Z * 3.0);

    assert!(cube.set_positions(&before));
    assert_eq!(cube.positions(), before.as_slice());
}

#[test]
fn a_short_set_is_refused() {
    // 🔴 The faces index these. A set one corner short would leave them
    // pointing past the end, and writing what fits would reshape the
    // block into something nobody authored.
    let mut cube = unit_cube();
    let before = cube.positions().to_vec();
    assert!(!cube.set_positions(&before[..7]));
    assert_eq!(cube.positions(), before.as_slice());
}
