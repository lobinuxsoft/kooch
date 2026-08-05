//! Single-LOD entry points. Every meshlet emitted is a DAG root
//! (`parent_meshlet_index == MESHLET_ROOT_PARENT`, `lod_error == 0`).
//! Use [`super::build_meshlets_lod_chain`] for multi-LOD output.

use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::asset::{DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MeshletMesh};

use super::common::{clusterize_lod, total_aabb};
use super::error::MeshletBuildError;

/// Builds a [`MeshletMesh`] from `mesh` using `meshopt`'s
/// clusterization. Single-LOD output: every meshlet is a DAG root
/// with `lod_error = 0`. Use [`super::build_meshlets_lod_chain`] to
/// produce a multi-LOD asset.
///
/// `max_vertices` and `max_triangles` cap each meshlet's size — the
/// defaults ([`DEFAULT_MAX_VERTICES`] / [`DEFAULT_MAX_TRIANGLES`])
/// match every mesh-shader-capable GPU's recommended limits.
///
/// `cone_weight` weights the spatial vs. cone-tightness cost during
/// clusterisation. `0.5` is a balanced default; `0.0` ignores cones
/// (meshlets cluster by spatial proximity only); `1.0` heavily favours
/// tight cones at the cost of spatial coherence.
pub fn build_meshlets_from_mesh(
    mesh: &Mesh,
    max_vertices: usize,
    max_triangles: usize,
    cone_weight: f32,
) -> Result<MeshletMesh, MeshletBuildError> {
    if mesh.indices.is_empty() {
        return Err(MeshletBuildError::EmptyMesh);
    }

    let vertex_stride = std::mem::size_of::<MeshVertex>();
    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let adapter = meshopt::VertexDataAdapter::new(vertex_bytes, vertex_stride, 0)?;

    let (descriptors, meshlet_vertices, meshlet_triangles) = clusterize_lod(
        &mesh.indices,
        &adapter,
        &mesh.vertices,
        max_vertices,
        max_triangles,
        cone_weight,
        0.0,
    );

    Ok(MeshletMesh {
        vertices: mesh.vertices.clone(),
        meshlet_vertices,
        meshlet_triangles,
        meshlets: descriptors,
        aabb: total_aabb(&mesh.vertices),
    })
}

/// Convenience wrapper using the default meshlet sizing
/// ([`DEFAULT_MAX_VERTICES`] = 64 verts, [`DEFAULT_MAX_TRIANGLES`] =
/// 124 tris) and a balanced `cone_weight` of `0.5`.
pub fn build_default_meshlets(mesh: &Mesh) -> Result<MeshletMesh, MeshletBuildError> {
    build_meshlets_from_mesh(mesh, DEFAULT_MAX_VERTICES, DEFAULT_MAX_TRIANGLES, 0.5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meshlet::asset::MESHLET_ROOT_PARENT;
    use crate::meshlet::builder::test_support::{make_grid_mesh, vertex};

    #[test]
    fn empty_mesh_returns_empty_error() {
        let mesh = Mesh::empty();
        let err = build_default_meshlets(&mesh).unwrap_err();
        assert!(matches!(err, MeshletBuildError::EmptyMesh));
    }

    #[test]
    fn single_triangle_yields_one_meshlet() {
        let mesh = Mesh::from_arrays(
            vec![
                vertex([0.0, 0.0, 0.0]),
                vertex([1.0, 0.0, 0.0]),
                vertex([0.0, 1.0, 0.0]),
            ],
            vec![0, 1, 2],
        );

        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.meshlet_count(), 1);
        assert_eq!(meshlet_mesh.meshlets[0].triangle_count, 1);
        assert!(meshlet_mesh.meshlets[0].vertex_count >= 3);
    }

    #[test]
    fn quad_yields_meshlet_covering_two_triangles() {
        // Quad: 4 vertices, 2 triangles.
        let mesh = Mesh::from_arrays(
            vec![
                vertex([0.0, 0.0, 0.0]),
                vertex([1.0, 0.0, 0.0]),
                vertex([1.0, 1.0, 0.0]),
                vertex([0.0, 1.0, 0.0]),
            ],
            vec![0, 1, 2, 0, 2, 3],
        );

        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.total_triangle_count(), 2);
        // Bounding sphere should cover the quad's extent.
        let m = &meshlet_mesh.meshlets[0];
        assert!(m.bounding_radius > 0.5);
    }

    #[test]
    fn total_aabb_covers_every_vertex() {
        let mesh = Mesh::from_arrays(
            vec![
                vertex([-2.0, -3.0, 1.0]),
                vertex([5.0, 4.0, 6.0]),
                vertex([0.0, 0.0, 0.0]),
            ],
            vec![0, 1, 2],
        );

        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.aabb.min, glam::Vec3::new(-2.0, -3.0, 0.0));
        assert_eq!(meshlet_mesh.aabb.max, glam::Vec3::new(5.0, 4.0, 6.0));
    }

    #[test]
    fn vertices_are_copied_into_pool() {
        let mesh = Mesh::from_arrays(
            vec![
                vertex([0.0, 0.0, 0.0]),
                vertex([1.0, 0.0, 0.0]),
                vertex([0.0, 1.0, 0.0]),
            ],
            vec![0, 1, 2],
        );
        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.total_vertex_count(), 3);
    }

    #[test]
    fn single_lod_meshes_keep_root_sentinel_and_zero_error() {
        // Default builder is unchanged: every meshlet a root, error 0.
        let mesh = make_grid_mesh(20);
        let single = build_default_meshlets(&mesh).expect("build");
        for m in &single.meshlets {
            assert_eq!(m.parent_meshlet_index, MESHLET_ROOT_PARENT);
            assert_eq!(m.lod_error, 0.0);
        }
    }
}
