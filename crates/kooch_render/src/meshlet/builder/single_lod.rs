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
mod tests;
