use glam::Vec3;

use super::{Adjacency, NO_FACE};
use crate::BlockMesh;

fn cube() -> BlockMesh {
    BlockMesh::cuboid(Vec3::splat(0.5))
}

/// One quad, alone: every edge is a boundary.
fn quad() -> BlockMesh {
    let positions = vec![
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
    ];
    BlockMesh::from_faces(positions, &[vec![0, 1, 2, 3]]).unwrap()
}

#[test]
fn a_cube_has_twelve_edges() {
    // Euler: 8 corners − 12 edges + 6 faces = 2.
    assert_eq!(Adjacency::of(&cube()).edge_count(), 12);
}

#[test]
fn every_cube_edge_joins_two_faces() {
    let adjacency = Adjacency::of(&cube());
    for edge in 0..adjacency.edge_count() as u32 {
        let faces: Vec<u32> = adjacency.faces_of(edge).collect();
        assert_eq!(faces.len(), 2, "edge {edge} has {faces:?}");
        assert_ne!(faces[0], faces[1], "an edge cannot have one face twice");
    }
}

#[test]
fn a_cube_is_closed() {
    assert!(Adjacency::of(&cube()).is_closed());
}

#[test]
fn a_cube_corner_touches_three_edges() {
    let adjacency = Adjacency::of(&cube());
    for corner in 0..8u32 {
        assert_eq!(adjacency.edges_at(corner).map(<[u32]>::len), Some(3));
    }
}

#[test]
fn a_face_names_its_own_edges() {
    let mesh = cube();
    let adjacency = Adjacency::of(&mesh);
    for face in 0..mesh.face_count() {
        let edges = adjacency.edges_of(face).expect("the face exists");
        assert_eq!(edges.len(), 4);
        // Every one of them agrees it touches this face.
        for edge in edges {
            assert!(
                adjacency
                    .faces_of(*edge)
                    .any(|other| other as usize == face),
                "face {face} claims edge {edge}, which does not claim it back",
            );
        }
    }
}

#[test]
fn a_faces_edges_follow_its_corners() {
    // Entry k is the edge LEAVING corner k, so the pair matches.
    let mesh = cube();
    let adjacency = Adjacency::of(&mesh);
    let corners = mesh.face(0).unwrap();
    let edges = adjacency.edges_of(0).unwrap();

    for step in 0..corners.len() {
        let from = corners[step];
        let to = corners[(step + 1) % corners.len()];
        let mut pair = adjacency.edge_corners(edges[step]).unwrap();
        pair.sort_unstable();
        let mut wanted = [from, to];
        wanted.sort_unstable();
        assert_eq!(pair, wanted, "edge {step} of face 0");
    }
}

#[test]
fn a_lone_quad_is_all_boundary() {
    let adjacency = Adjacency::of(&quad());
    assert_eq!(adjacency.edge_count(), 4);
    for edge in 0..4u32 {
        assert!(adjacency.is_boundary(edge));
        assert_eq!(adjacency.edge_faces(edge), Some([0, NO_FACE]));
    }
}

#[test]
fn a_lone_quad_is_manifold_but_open() {
    // An open mesh is legitimate. A collider built from one is not.
    let adjacency = Adjacency::of(&quad());
    assert!(adjacency.is_manifold());
    assert!(!adjacency.is_closed());
}

#[test]
fn a_third_face_is_not_manifold() {
    // A fan: three quads sharing one edge. Nothing rejects it at the
    // door, and silently keeping two of the three is how it becomes a
    // mesh that looks fine and extrudes wrong.
    let positions = vec![
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(1.0, 1.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
        Vec3::new(-1.0, 1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(0.0, 1.0, 1.0),
    ];
    let mesh = BlockMesh::from_faces(
        positions,
        &[vec![0, 1, 3, 2], vec![0, 1, 5, 4], vec![0, 1, 7, 6]],
    )
    .unwrap();

    assert!(!Adjacency::of(&mesh).is_manifold());
}

#[test]
fn an_empty_mesh_has_no_edges() {
    let adjacency = Adjacency::of(&BlockMesh::default());
    assert_eq!(adjacency.edge_count(), 0);
    assert!(adjacency.edges_of(0).is_none());
    // Vacuously: nothing is crowded and no edge is a boundary.
    assert!(adjacency.is_manifold());
}

#[test]
fn an_unknown_edge_answers_none() {
    let adjacency = Adjacency::of(&cube());
    assert!(adjacency.edge_corners(99).is_none());
    assert!(adjacency.edge_faces(99).is_none());
    assert!(adjacency.edges_at(99).is_none());
}
