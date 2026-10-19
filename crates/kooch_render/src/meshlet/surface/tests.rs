use super::*;
use crate::mesh::{Aabb, MeshVertex};
use crate::meshlet::MeshletDescriptor;

/// `Pod`, so all-zero is a legal value and a test names only what it cares about.
fn blank() -> MeshletDescriptor {
    unsafe { std::mem::zeroed() }
}

/// One triangle at LOD 0 and the same surface again at LOD 1 — what every simplified mesh
/// looks like.
fn two_levels() -> MeshletMesh {
    let vertices = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]
        .map(|position| MeshVertex {
            position,
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
        })
        .to_vec();
    let fine = MeshletDescriptor {
        vertex_count: 3,
        triangle_count: 1,
        lod_level: 0,
        ..blank()
    };
    MeshletMesh {
        vertices,
        meshlet_vertices: vec![0, 1, 2, 0, 1, 2],
        meshlet_triangles: vec![0, 1, 2, 0, 1, 2],
        meshlets: vec![
            fine,
            MeshletDescriptor {
                vertex_offset: 3,
                triangle_offset: 3,
                lod_level: 1,
                ..fine
            },
        ],
        aabb: Aabb::empty(),
    }
}

/// 🔴 A collider built from every level is the same surface several times over — a solid full
/// of overlapping shells that a character catches on, with nothing anywhere reporting a fault.
#[test]
fn only_the_finest_level_collides() {
    assert_eq!(base_triangles(&two_levels()), vec![[0, 1, 2]]);
}
