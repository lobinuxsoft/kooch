//! Engine geometry as rapier geometry, the only place knowing both. A shape rapier refuses
//! (collinear hull, degenerate trimesh, empty voxels) returns the refusal: the body builds without
//! it and logs, never a secret ball.

use rapier3d::parry::transformation::voxelization::FillMode;
use rapier3d::parry::utils::Array2;
use rapier3d::prelude::*;

use glam::Vec3;

use crate::backend::{CollisionShape, ConvexPart, MIN_EXTENT};

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
        CollisionShape::ConvexHull { part } => convex(part),
        CollisionShape::ConvexDecomposition { vertices, indices } => {
            non_empty(vertices)?;
            Ok(ColliderBuilder::convex_decomposition(vertices, indices))
        }
        CollisionShape::Compound { parts } => compound(parts),
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

/// Solid interior or shell; shells let a body inside pass through, so fill is the default.
fn fill_mode(solid: bool) -> FillMode {
    match solid {
        true => FillMode::FloodFill {
            detect_cavities: false,
        },
        false => FillMode::SurfaceOnly,
    }
}

/// One convex piece, hulled only when nobody vouched for its faces ([`ConvexPart`]).
fn convex(part: &ConvexPart) -> Result<ColliderBuilder, ShapeError> {
    non_empty(&part.points)?;
    match part.is_hulled() {
        true => ColliderBuilder::convex_mesh(part.points.clone(), &part.faces)
            .ok_or(ShapeError::DegenerateHull),
        false => ColliderBuilder::convex_hull(&part.points).ok_or(ShapeError::DegenerateHull),
    }
}

/// One collider from convex pieces, each already in body space, so every pose is identity.
fn compound(parts: &[ConvexPart]) -> Result<ColliderBuilder, ShapeError> {
    if parts.is_empty() {
        return Err(ShapeError::NoGeometry);
    }
    let mut shapes = Vec::with_capacity(parts.len());
    for part in parts {
        // Where the saving is largest: a decomposition is twelve pieces
        // for Suzanne and twenty-six for the dragon, and every one of
        // them would otherwise be hulled again on every body build.
        let shape = convex(part)?.build().shared_shape().clone();
        shapes.push((Pose::IDENTITY, shape));
    }
    Ok(ColliderBuilder::compound(shapes))
}

/// A cloud's convex hull as points and triangles — a 76 038-vertex dragon becomes 387. `None`
/// without volume, as `shape_builder` refuses.
pub fn hull_of(points: &[Vec3]) -> Option<(Vec<Vec3>, Vec<[u32; 3]>)> {
    if points.len() < 4 {
        return None;
    }
    let (hull, faces) = rapier3d::parry::transformation::convex_hull(points);
    // Checked on output too: parry returns something for coplanar clouds; a tetrahedron is the
    // smallest volume.
    match hull.len() >= 4 && faces.len() >= 4 {
        true => Some((hull, faces)),
        false => None,
    }
}

/// A concave mesh split by VHACD — 1.35 s for Suzanne, 2.58 s for a 76k dragon, hence baking.
/// Pieces come back as clouds for [`CollisionShape::Compound`].
pub fn decompose(vertices: &[Vec3], indices: &[[u32; 3]]) -> Vec<Vec<Vec3>> {
    use rapier3d::parry::transformation::vhacd::{VHACD, VHACDParameters};

    if vertices.len() < 4 || indices.is_empty() {
        return Vec::new();
    }
    VHACD::decompose(&VHACDParameters::default(), vertices, indices, true)
        // One convex hull per part, at the finest downsampling — the
        // pieces are the product, and a coarser hull of each would give
        // back volume the decomposition just spent seconds removing.
        .compute_convex_hulls(1)
        .into_iter()
        .map(|(points, _)| points)
        .collect()
}

/// The height grid checked against its dimensions, since `Array2::new` asserts inside the solver.
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

/// Logs a refused shape at `error`: nothing compensates, and the body collides with nothing.
pub(super) fn warn_refused(shape: &CollisionShape, error: &ShapeError) {
    tracing::error!(
        target: "kooch_physics::shape",
        shape = shape.name(),
        "collider not built: {error}",
    );
}

#[cfg(test)]
mod tests;
