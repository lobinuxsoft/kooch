//! `Mesh` — CPU-side asset type loaded from glTF.
//!
//! A `Mesh` holds geometry data parsed from disk. It is the type
//! `GltfMeshLoader` produces and the type [`Assets<Mesh>`] stores. The
//! meshlet builder consumes it directly to produce GPU-resident
//! `MeshletMesh` data — there is no separate raw-mesh GPU upload.

use glam::Vec3;

use super::vertex::{Aabb, MeshVertex};

/// CPU-side mesh data: interleaved vertex array + 32-bit indices + AABB.
///
/// Layout matches [`MeshVertex`] for one-shot upload via
/// `bytemuck::cast_slice`. Loaders are responsible for filling missing
/// attributes (normals, uvs) with sensible defaults so the GPU layout stays
/// consistent across assets.
#[derive(Debug, Clone)]
pub struct Mesh {
    /// Interleaved vertex stream (position + normal + uv).
    pub vertices: Vec<MeshVertex>,
    /// Index list (always `u32` for simplicity — 16-bit upcast at parse time).
    pub indices: Vec<u32>,
    /// Local-space bounds, computed from positions during load.
    pub aabb: Aabb,
}

impl Mesh {
    /// A mesh from bare positions and triangles.
    ///
    /// What the engine's own generators produce: a convex hull, a
    /// decomposed piece. Normals are accumulated from the faces so the
    /// result is shaded rather than black when an artist opens it, and
    /// UVs are zero because a collision proxy has nothing to map.
    pub fn from_triangles(positions: &[glam::Vec3], triangles: &[[u32; 3]]) -> Self {
        let mut normals = vec![glam::Vec3::ZERO; positions.len()];
        for tri in triangles {
            let [a, b, c] = tri.map(|i| positions[i as usize]);
            // Unnormalised on purpose: the cross product's length is
            // twice the triangle's area, which weights a big face more
            // than a sliver — the standard accumulation.
            let face = (b - a).cross(c - a);
            for index in tri {
                normals[*index as usize] += face;
            }
        }

        let mut aabb = Aabb::empty();
        let vertices = positions
            .iter()
            .zip(&normals)
            .map(|(position, normal)| {
                aabb.expand(*position);
                MeshVertex {
                    position: position.to_array(),
                    normal: normal.normalize_or(glam::Vec3::Y).to_array(),
                    uv: [0.0, 0.0],
                }
            })
            .collect();

        Self {
            vertices,
            indices: triangles.iter().flatten().copied().collect(),
            aabb,
        }
    }

    /// Empty mesh (no vertices, no indices). Useful as a placeholder while
    /// async loads complete.
    pub fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            aabb: Aabb::empty(),
        }
    }

    /// Builds a mesh from raw streams. Computes AABB from positions.
    pub fn from_arrays(vertices: Vec<MeshVertex>, indices: Vec<u32>) -> Self {
        let mut aabb = Aabb::empty();
        for v in &vertices {
            aabb.expand(Vec3::from_array(v.position));
        }
        Self {
            vertices,
            indices,
            aabb,
        }
    }

    /// Vertex count.
    pub fn vertex_count(&self) -> u32 {
        self.vertices.len() as u32
    }

    /// Index count.
    pub fn index_count(&self) -> u32 {
        self.indices.len() as u32
    }
}

impl Default for Mesh {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests;
