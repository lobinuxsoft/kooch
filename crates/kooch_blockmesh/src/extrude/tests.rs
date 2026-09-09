use glam::Vec3;

use crate::{Adjacency, BlockMesh};

fn cube() -> BlockMesh {
    BlockMesh::cuboid(Vec3::splat(0.5))
}

/// The face whose outward normal points along `axis`.
fn facing(mesh: &BlockMesh, axis: Vec3) -> u32 {
    (0..mesh.face_count())
        .find(|face| mesh.face_normal(*face).unwrap().dot(axis) > 0.99)
        .expect("a cube faces every axis") as u32
}

/// Every generated normal points away from the mesh's middle.
fn all_outward(mesh: &BlockMesh) -> Vec<usize> {
    let centre: Vec3 = mesh.positions().iter().sum::<Vec3>() / mesh.positions().len() as f32;
    (0..mesh.face_count())
        .filter(|face| {
            let corners = mesh.face(*face).unwrap();
            let middle: Vec3 = corners
                .iter()
                .map(|c| mesh.positions()[*c as usize])
                .sum::<Vec3>()
                / corners.len() as f32;
            mesh.face_normal(*face).unwrap().dot(middle - centre) <= 0.0
        })
        .collect()
}

#[test]
fn one_face_grows_four_walls() {
    let mut mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    let out = mesh.extrude(&[front], Vec3::Z).expect("it extrudes");

    assert_eq!(out.walls.len(), 4, "a quad's rim is four edges");
    assert_eq!(mesh.face_count(), 10, "six faces plus four walls");
}

#[test]
fn the_extruded_face_travels() {
    let mut mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    mesh.extrude(&[front], Vec3::Z * 2.0).unwrap();

    let centre = mesh.centre_of(&[front]).unwrap();
    assert!((centre.z - 2.5).abs() < 1e-4, "{centre}");
}

#[test]
fn the_rest_of_the_block_stays() {
    let mut mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    let back = facing(&mesh, -Vec3::Z);
    let before = mesh.centre_of(&[back]).unwrap();

    mesh.extrude(&[front], Vec3::Z * 2.0).unwrap();

    assert_eq!(mesh.centre_of(&[back]).unwrap(), before);
}

#[test]
fn the_walls_face_outward() {
    let mut mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    mesh.extrude(&[front], Vec3::Z * 2.0).unwrap();

    let inward = all_outward(&mesh);
    assert!(inward.is_empty(), "these faces point inward: {inward:?}");
}

#[test]
fn an_extruded_block_is_still_closed() {
    // 🔴 The whole point of stitching. A rim left open is a hole the
    // renderer draws through and the solver lets things fall into.
    let mut mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    mesh.extrude(&[front], Vec3::Z * 2.0).unwrap();

    let adjacency = Adjacency::of(&mesh);
    assert!(adjacency.is_manifold(), "an edge gained a third face");
    assert!(adjacency.is_closed(), "the extrusion left a hole");
}

#[test]
fn two_faces_extrude_as_one_piece() {
    // 🔴 The case the feature exists for. Two adjacent faces pulled
    // together are ONE box, not two pillars: their shared edge is
    // interior and gets no wall.
    let mut mesh = cube();
    let faces = [facing(&mesh, Vec3::Z), facing(&mesh, Vec3::X)];
    let out = mesh.extrude(&faces, Vec3::Y).expect("it extrudes");

    // Six rim edges around an L of two quads, not eight.
    assert_eq!(out.walls.len(), 6, "the shared edge grew a wall");
    assert!(Adjacency::of(&mesh).is_manifold());
}

#[test]
fn a_shared_edge_gets_no_wall() {
    let mut mesh = cube();
    let faces = [facing(&mesh, Vec3::Z), facing(&mesh, Vec3::X)];
    let before = mesh.face_count();
    mesh.extrude(&faces, Vec3::Y).unwrap();

    // Two quads, eight edges, two of which are the one they share.
    assert_eq!(mesh.face_count(), before + 6);
}

#[test]
fn the_selection_follows_the_faces() {
    // So a second extrude continues the wall rather than starting
    // beside it.
    let mut mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    let out = mesh.extrude(&[front], Vec3::Z).unwrap();
    assert_eq!(out.faces, vec![front]);

    let again = mesh.extrude(&out.faces, Vec3::Z).unwrap();
    assert_eq!(again.walls.len(), 4);
    assert!(Adjacency::of(&mesh).is_closed());
}

#[test]
fn a_zero_extrude_still_splits() {
    // ProBuilder's "extrude then drag": the topology is made first and
    // the distance comes from the handle.
    let mut mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    let before = mesh.face_count();
    let out = mesh.extrude(&[front], Vec3::ZERO).unwrap();

    assert_eq!(out.walls.len(), 4);
    assert_eq!(mesh.face_count(), before + 4);
}

#[test]
fn extruding_nothing_answers_none() {
    assert!(cube().extrude(&[], Vec3::Z).is_none());
    assert!(cube().extrude(&[99], Vec3::Z).is_none());
}

#[test]
fn every_face_still_names_real_corners() {
    let mut mesh = cube();
    mesh.extrude(&[facing(&mesh, Vec3::Z)], Vec3::Z).unwrap();

    let corners = mesh.positions().len() as u32;
    for face in mesh.faces() {
        assert!(face.iter().all(|c| *c < corners), "a face ran off the end");
        assert!(face.len() >= 3);
    }
}

#[test]
fn one_face_extrudes_along_its_normal() {
    let mesh = cube();
    let front = facing(&mesh, Vec3::Z);
    let by = mesh.extrude_direction(&[front], 2.0).unwrap();
    assert!((by - Vec3::Z * 2.0).length() < 1e-4, "{by}");
}

#[test]
fn two_faces_share_one_direction() {
    // 🔴 Averaged. Per-face would tear a patch into a fan of pieces
    // with nowhere for the walls between them to meet.
    let mesh = cube();
    let faces = [facing(&mesh, Vec3::Z), facing(&mesh, Vec3::X)];
    let by = mesh.extrude_direction(&faces, 1.0).unwrap();

    let wanted = (Vec3::Z + Vec3::X).normalize();
    assert!((by - wanted).length() < 1e-4, "{by}");
}

#[test]
fn opposite_faces_have_no_direction() {
    // No shared "out". Picking one would extrude half the selection
    // backwards, which is worse than refusing.
    let mesh = cube();
    let faces = [facing(&mesh, Vec3::Z), facing(&mesh, -Vec3::Z)];
    assert!(mesh.extrude_direction(&faces, 1.0).is_none());
}

#[test]
fn nothing_selected_has_no_direction() {
    assert!(cube().extrude_direction(&[], 1.0).is_none());
}
