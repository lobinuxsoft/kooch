//! [`ShapeSpec`]: a collider's geometry as thirteen `Copy` fields, and the one place it becomes a
//! [`CollisionShape`] — comparing resolved geometry would diff a trimesh per body per frame.
//! `mesh_epoch` makes an arriving mesh a change.

use glam::Vec3;

use crate::backend::{ColliderMesh, ColliderMeshCache, CollisionShape, MIN_EXTENT};

use super::shapes::*;

/// A collider's geometry, comparable by value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeSpec {
    pub shape: u32,
    pub radius: f32,
    pub half_extents: Vec3,
    pub half_height: f32,
    pub border_radius: f32,
    pub normal: Vec3,
    pub point_a: Vec3,
    pub point_b: Vec3,
    pub point_c: Vec3,
    pub voxel_size: f32,
    pub voxel_solid: bool,
    /// Where the geometry comes from. 🔴 Not a bare `Guid`: a generated mesh has no file.
    pub mesh: Option<crate::backend::MeshKey>,
    /// What [`ColliderMeshCache::epoch`] said when this spec was read.
    pub mesh_epoch: u64,
}

impl ShapeSpec {
    /// The geometry, or `None` while a mesh shape waits. Degenerate numbers are clamped (edits pass
    /// through zero); a missing mesh is not replaced by a sphere nobody authored.
    pub fn resolve(&self, meshes: Option<&ColliderMeshCache>) -> Option<CollisionShape> {
        if is_mesh_derived(self.shape) {
            return self.from_mesh(self.mesh_data(meshes)?);
        }
        Some(self.analytic())
    }

    /// `true` when this shape names a mesh it has not been given.
    pub fn awaits_mesh(&self, meshes: Option<&ColliderMeshCache>) -> bool {
        is_mesh_derived(self.shape) && self.mesh_data(meshes).is_none()
    }

    /// Shapes from typed numbers; unknown discriminants fall back to a sphere so newer scenes still
    /// collide.
    fn analytic(&self) -> CollisionShape {
        let radius = self.radius.max(MIN_EXTENT);
        let half_height = self.half_height.max(MIN_EXTENT);
        match self.shape {
            SHAPE_CUBOID => CollisionShape::Cuboid {
                half_extents: self.half_extents.max(Vec3::splat(MIN_EXTENT)),
            },
            SHAPE_CAPSULE => CollisionShape::Capsule {
                radius,
                half_height,
            },
            SHAPE_CYLINDER => CollisionShape::Cylinder {
                radius,
                half_height,
            },
            SHAPE_ROUND_CYLINDER => CollisionShape::RoundCylinder {
                radius,
                half_height,
                border_radius: self.border_radius.max(MIN_EXTENT),
            },
            SHAPE_CONE => CollisionShape::Cone {
                radius,
                half_height,
            },
            SHAPE_HALF_SPACE => CollisionShape::HalfSpace {
                normal: self.normal,
            },
            SHAPE_SEGMENT => CollisionShape::Segment {
                a: self.point_a,
                b: self.point_b,
            },
            SHAPE_TRIANGLE => CollisionShape::Triangle {
                a: self.point_a,
                b: self.point_b,
                c: self.point_c,
            },
            _ => CollisionShape::Sphere { radius },
        }
    }

    /// The mesh behind a mesh-derived shape, if one has arrived.
    fn mesh_data<'a>(&self, meshes: Option<&'a ColliderMeshCache>) -> Option<&'a ColliderMesh> {
        let mesh = meshes?.get(self.mesh?)?;
        match mesh.is_empty() {
            true => None,
            false => Some(mesh),
        }
    }

    /// Shapes from the mesh's points; `None` when it cannot supply them, often a hull-only cloud.
    fn from_mesh(&self, mesh: &ColliderMesh) -> Option<CollisionShape> {
        let size = self.voxel_size.max(MIN_EXTENT);
        let shape = match self.shape {
            SHAPE_CONVEX_HULL => CollisionShape::ConvexHull {
                part: mesh.hull_or_vertices(),
            },
            SHAPE_POLYLINE => CollisionShape::Polyline {
                vertices: mesh.vertices.clone(),
            },
            SHAPE_VOXELS => CollisionShape::voxels_from_points(Vec3::splat(size), &mesh.vertices),
            // A baked decomposition is already the pieces, and skipping
            // VHACD is the difference between milliseconds and seconds.
            SHAPE_CONVEX_DECOMPOSITION if !mesh.parts.is_empty() => CollisionShape::Compound {
                parts: mesh.parts.clone(),
            },
            SHAPE_CONVEX_DECOMPOSITION => CollisionShape::ConvexDecomposition {
                vertices: mesh.vertices.clone(),
                indices: non_empty(&mesh.indices)?.to_vec(),
            },
            // The entity's own triangles, built as a trimesh — only the source differs.
            SHAPE_OWN_MESH => CollisionShape::TriMesh {
                vertices: mesh.vertices.clone(),
                indices: non_empty(&mesh.indices)?.to_vec(),
            },
            SHAPE_TRIMESH => CollisionShape::TriMesh {
                vertices: mesh.vertices.clone(),
                indices: non_empty(&mesh.indices)?.to_vec(),
            },
            SHAPE_VOXELIZED_MESH => CollisionShape::VoxelizedMesh {
                vertices: mesh.vertices.clone(),
                indices: non_empty(&mesh.indices)?.to_vec(),
                size,
                solid: self.voxel_solid,
            },
            _ => return None,
        };
        Some(shape)
    }
}

fn non_empty(indices: &[[u32; 3]]) -> Option<&[[u32; 3]]> {
    match indices.is_empty() {
        true => None,
        false => Some(indices),
    }
}

#[cfg(test)]
mod tests;
