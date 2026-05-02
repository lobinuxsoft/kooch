//! `Mesh` → `MeshletMesh` offline builder using `meshopt`.

use glam::Vec3;

use crate::mesh::{Aabb, Mesh};

use super::asset::{MeshletDescriptor, MeshletMesh, DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES};

/// Errors raised while building a meshlet mesh.
#[derive(Debug)]
pub enum MeshletBuildError {
    /// Source mesh had no triangles.
    EmptyMesh,
    /// `meshopt` rejected the vertex layout (stride mismatch, etc.).
    VertexAdapter(meshopt::Error),
}

impl std::fmt::Display for MeshletBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyMesh => write!(f, "cannot build meshlets from a mesh with zero triangles"),
            Self::VertexAdapter(e) => write!(f, "meshopt vertex adapter failed: {e}"),
        }
    }
}

impl std::error::Error for MeshletBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::VertexAdapter(e) => Some(e),
            _ => None,
        }
    }
}

impl From<meshopt::Error> for MeshletBuildError {
    fn from(e: meshopt::Error) -> Self {
        Self::VertexAdapter(e)
    }
}

/// Builds a [`MeshletMesh`] from `mesh` using `meshopt`'s clusterization.
///
/// `max_vertices` and `max_triangles` cap each meshlet's size — the
/// defaults ([`DEFAULT_MAX_VERTICES`] / [`DEFAULT_MAX_TRIANGLES`]) match
/// every mesh-shader-capable GPU's recommended limits.
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

    let vertex_stride = std::mem::size_of::<crate::mesh::MeshVertex>();
    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let adapter = meshopt::VertexDataAdapter::new(vertex_bytes, vertex_stride, 0)?;

    let raw = meshopt::build_meshlets(
        &mesh.indices,
        &adapter,
        max_vertices,
        max_triangles,
        cone_weight,
    );

    // `meshopt` returns: meshlets[], a flat vertex index pool, a flat
    // triangle index pool. Each meshlet's vertex_offset/triangle_offset
    // index those pools.
    let mut descriptors = Vec::with_capacity(raw.len());
    for i in 0..raw.len() {
        let m = raw.get(i);
        let bounds = meshopt::compute_meshlet_bounds(m, &adapter);
        // `meshopt_Meshlet` (the ffi struct) carries the raw offsets
        // into `raw.vertices` / `raw.triangles`; the safe `Meshlet`
        // wrapper reslices these for us. We need both: descriptors
        // store the offsets (so the GPU can index the flat pool),
        // and the AABB pass uses the meshlet's vertex set directly.
        let ffi = &raw.meshlets[i];
        let (aabb_min, aabb_max) = meshlet_aabb(m, &mesh.vertices);
        descriptors.push(MeshletDescriptor {
            vertex_offset: ffi.vertex_offset,
            triangle_offset: ffi.triangle_offset,
            vertex_count: ffi.vertex_count,
            triangle_count: ffi.triangle_count,
            aabb_min,
            _pad0: 0,
            aabb_max,
            _pad1: 0,
            cone_apex: bounds.center,
            bounding_radius: bounds.radius,
            cone_axis: bounds.cone_axis,
            cone_cutoff: bounds.cone_cutoff,
        });
    }

    let mut total_aabb = Aabb::empty();
    for v in &mesh.vertices {
        total_aabb.expand(Vec3::from_array(v.position));
    }

    Ok(MeshletMesh {
        vertices: mesh.vertices.clone(),
        meshlet_vertices: raw.vertices,
        meshlet_triangles: raw.triangles,
        meshlets: descriptors,
        aabb: total_aabb,
    })
}

/// Per-meshlet AABB, computed from the meshlet's vertex slice. The
/// meshlet's `vertices` slice stores indices INTO the parent mesh's
/// vertex array, so we look up positions there directly.
fn meshlet_aabb(
    meshlet: meshopt::Meshlet<'_>,
    vertex_pool: &[crate::mesh::MeshVertex],
) -> ([f32; 3], [f32; 3]) {
    let mut mn = [f32::INFINITY; 3];
    let mut mx = [f32::NEG_INFINITY; 3];
    for &pool_idx in meshlet.vertices {
        let p = vertex_pool[pool_idx as usize].position;
        for axis in 0..3 {
            if p[axis] < mn[axis] {
                mn[axis] = p[axis];
            }
            if p[axis] > mx[axis] {
                mx[axis] = p[axis];
            }
        }
    }
    (mn, mx)
}

/// Convenience wrapper using the default meshlet sizing
/// ([`DEFAULT_MAX_VERTICES`] = 64 verts, [`DEFAULT_MAX_TRIANGLES`] = 124
/// tris) and a balanced `cone_weight` of `0.5`.
pub fn build_default_meshlets(mesh: &Mesh) -> Result<MeshletMesh, MeshletBuildError> {
    build_meshlets_from_mesh(
        mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::MeshVertex;

    fn vertex(p: [f32; 3]) -> MeshVertex {
        MeshVertex {
            position: p,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        }
    }

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
}
