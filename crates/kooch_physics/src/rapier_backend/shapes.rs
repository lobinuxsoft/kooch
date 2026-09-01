//! Engine geometry, as rapier geometry.
//!
//! The only place in the crate that knows both vocabularies, and the
//! reason [`CollisionShape`] can grow a variant without anything outside
//! this file learning a rapier type.
//!
//! # A shape that cannot be built says so
//!
//! Three of rapier's constructors can refuse: a convex hull of collinear
//! points has no volume, a trimesh can be degenerate, and a voxel set can
//! come out empty. Rapier answers `None` or an `Err`, and the tempting
//! move is to substitute a small ball so the call site keeps its
//! signature — which produces a collider nobody authored, in a place
//! nobody looks. So this returns the refusal, and the backend logs it and
//! builds the body without that shape: a body that visibly does not
//! collide is a bug report, and a secret ball is not.

use rapier3d::parry::transformation::voxelization::FillMode;
use rapier3d::parry::utils::Array2;
use rapier3d::prelude::*;

use glam::Vec3;

use crate::backend::{CollisionShape, MIN_EXTENT};

/// Why a shape could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ShapeError {
    /// The point set has no volume — collinear, coplanar, or too few
    /// points for a hull.
    DegenerateHull,
    /// Rapier rejected the triangles.
    BrokenTriMesh,
    /// A mesh-derived shape arrived with nothing in it.
    NoGeometry,
    /// The height grid's length does not match `rows × cols`.
    RaggedHeightfield,
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::DegenerateHull => {
                "the points have no volume, so a convex hull cannot be built from them"
            }
            Self::BrokenTriMesh => "rapier rejected the triangle mesh",
            Self::NoGeometry => "the mesh has no vertices",
            Self::RaggedHeightfield => "the height grid does not match its rows and columns",
        };
        f.write_str(text)
    }
}

/// The shape, before density or placement.
pub(super) fn shape_builder(shape: &CollisionShape) -> Result<ColliderBuilder, ShapeError> {
    match shape {
        CollisionShape::Sphere { radius } => Ok(ColliderBuilder::ball(dim(*radius))),
        CollisionShape::Cuboid { half_extents } => Ok(ColliderBuilder::cuboid(
            dim(half_extents.x),
            dim(half_extents.y),
            dim(half_extents.z),
        )),
        CollisionShape::Capsule {
            radius,
            half_height,
        } => Ok(ColliderBuilder::capsule_y(dim(*half_height), dim(*radius))),
        CollisionShape::Cylinder {
            radius,
            half_height,
        } => Ok(ColliderBuilder::cylinder(dim(*half_height), dim(*radius))),
        CollisionShape::RoundCylinder {
            radius,
            half_height,
            border_radius,
        } => Ok(ColliderBuilder::round_cylinder(
            dim(*half_height),
            dim(*radius),
            dim(*border_radius),
        )),
        CollisionShape::Cone {
            radius,
            half_height,
        } => Ok(ColliderBuilder::cone(dim(*half_height), dim(*radius))),
        // Built from the shape rather than `ColliderBuilder::halfspace`,
        // whose `Unit` argument is nalgebra's — a type this crate has no
        // other reason to name.
        CollisionShape::HalfSpace { normal } => Ok(ColliderBuilder::new(SharedShape::halfspace(
            normal.normalize_or(Vec3::Y),
        ))),
        CollisionShape::Segment { a, b } => Ok(ColliderBuilder::segment(*a, *b)),
        CollisionShape::Triangle { a, b, c } => Ok(ColliderBuilder::triangle(*a, *b, *c)),
        CollisionShape::ConvexHull { points } => {
            non_empty(points)?;
            ColliderBuilder::convex_hull(points).ok_or(ShapeError::DegenerateHull)
        }
        CollisionShape::ConvexDecomposition { vertices, indices } => {
            non_empty(vertices)?;
            Ok(ColliderBuilder::convex_decomposition(vertices, indices))
        }
        CollisionShape::TriMesh { vertices, indices } => {
            non_empty(vertices)?;
            ColliderBuilder::trimesh(vertices.clone(), indices.clone())
                .map_err(|_| ShapeError::BrokenTriMesh)
        }
        CollisionShape::Polyline { vertices } => {
            non_empty(vertices)?;
            Ok(ColliderBuilder::polyline(vertices.clone(), None))
        }
        CollisionShape::Heightfield {
            heights,
            rows,
            cols,
            scale,
        } => heightfield(heights, *rows, *cols, *scale),
        CollisionShape::Voxels { size, cells } => match cells.is_empty() {
            true => Err(ShapeError::NoGeometry),
            false => Ok(ColliderBuilder::voxels(*size, cells)),
        },
        CollisionShape::VoxelizedMesh {
            vertices,
            indices,
            size,
            solid,
        } => {
            non_empty(vertices)?;
            Ok(ColliderBuilder::voxelized_mesh(
                vertices,
                indices,
                dim(*size),
                fill_mode(*solid),
            ))
        }
    }
}

/// A dimension the solver can build with.
fn dim(value: f32) -> f32 {
    value.max(MIN_EXTENT)
}

fn non_empty(points: &[Vec3]) -> Result<(), ShapeError> {
    match points.is_empty() {
        true => Err(ShapeError::NoGeometry),
        false => Ok(()),
    }
}

/// Whether the interior is solid or only the surface shell is.
///
/// A shell is what a hollow prop wants and what a body dropped *inside*
/// the shape passes straight through; the flood fill is the default
/// everywhere else.
fn fill_mode(solid: bool) -> FillMode {
    match solid {
        true => FillMode::FloodFill {
            detect_cavities: false,
        },
        false => FillMode::SurfaceOnly,
    }
}

/// The height grid, checked against its own dimensions.
///
/// `Array2::new` asserts the length matches, and an assert inside the
/// solver is a panic with no author-facing cause. Checked here so a
/// mis-sized grid is a refusal with a name.
fn heightfield(
    heights: &[f32],
    rows: u32,
    cols: u32,
    scale: Vec3,
) -> Result<ColliderBuilder, ShapeError> {
    let (rows, cols) = (rows as usize, cols as usize);
    if rows == 0 || cols == 0 || heights.len() != rows * cols {
        return Err(ShapeError::RaggedHeightfield);
    }
    Ok(ColliderBuilder::heightfield(
        Array2::new(rows, cols, heights.to_vec()),
        scale.max(Vec3::splat(MIN_EXTENT)),
    ))
}

/// Says which shape the solver would not take, and why.
///
/// At `error` rather than `warn`: nothing downstream compensates, so the
/// body is in the scene and collides with nothing until someone changes
/// the authored data.
pub(super) fn warn_refused(shape: &CollisionShape, error: &ShapeError) {
    tracing::error!(
        target: "kooch_physics::shape",
        shape = shape.name(),
        "collider not built: {error}",
    );
}

#[cfg(test)]
mod tests;
