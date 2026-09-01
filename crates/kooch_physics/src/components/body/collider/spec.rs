//! [`ShapeSpec`] — a collider's geometry as plain old data, and the one
//! place that turns it into a [`CollisionShape`].
//!
//! # Why the spec exists at all
//!
//! The sync pass rebuilds a body when its authored shape changes, and it
//! decides that by comparing what it built against what the Inspector
//! says. Comparing resolved geometry would mean holding a level's trimesh
//! per body and diffing it every frame. This is the same information in
//! thirteen `Copy` fields — including `mesh_epoch`, which is what makes a
//! mesh *arriving* register as a change rather than as silence.

use glam::Vec3;

use kooch_core::Guid;

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
    pub mesh: Option<Guid>,
    /// What [`ColliderMeshCache::epoch`] said when this spec was read.
    pub mesh_epoch: u64,
}

impl ShapeSpec {
    /// The geometry, or `None` when a mesh-derived shape has no mesh yet.
    ///
    /// Degenerate numbers are clamped rather than rejected: a field
    /// mid-edit passes through zero on the way to the value the author is
    /// typing, and a zero-radius shape makes the solver produce NaNs that
    /// outlive the typo. A *missing mesh* is the opposite case and stays
    /// `None` — substituting a unit sphere for a level's collision would
    /// be a floor nobody authored, in a place nobody looks.
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

    /// The shapes built from typed numbers alone.
    ///
    /// An unknown discriminant falls back to a sphere: a scene authored
    /// in a newer editor loads and collides with something, rather than
    /// dropping its colliders on the floor.
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

    /// The shapes built from that mesh's points.
    ///
    /// `None` where the mesh cannot supply what the shape needs — a
    /// decomposition or a trimesh with no triangles, most often a point
    /// cloud that was only ever meant to feed a hull.
    fn from_mesh(&self, mesh: &ColliderMesh) -> Option<CollisionShape> {
        let size = self.voxel_size.max(MIN_EXTENT);
        let shape = match self.shape {
            SHAPE_CONVEX_HULL => CollisionShape::ConvexHull {
                points: mesh.vertices.clone(),
            },
            SHAPE_POLYLINE => CollisionShape::Polyline {
                vertices: mesh.vertices.clone(),
            },
            SHAPE_VOXELS => CollisionShape::voxels_from_points(Vec3::splat(size), &mesh.vertices),
            SHAPE_CONVEX_DECOMPOSITION => CollisionShape::ConvexDecomposition {
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
