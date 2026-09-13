//! [`CollisionShape`]: plain glam geometry, never a backend type or asset handle — mesh shapes
//! carry points via [`ColliderMeshCache`](super::ColliderMeshCache). Not `Copy`, since hulls and
//! trimeshes are large.

use glam::{IVec3, Vec3};

/// Smallest built dimension: an Inspector edit through zero would make the solver produce NaNs that
/// outlive the typo.
pub const MIN_EXTENT: f32 = 1e-4;

/// Convex points with the faces that prove it. qhull costs 162 µs for a 226-point hull just to
/// return it, so `faces` claims the hull is done and the backend builds directly.
/// 🔴 Trusted, not checked — made only by qhull's output or an engine-baked asset.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConvexPart {
    pub points: Vec<Vec3>,
    /// The hull's triangles, or empty when nobody has vouched for them.
    pub faces: Vec<[u32; 3]>,
}

impl ConvexPart {
    /// A point cloud with no claim about it.
    pub fn loose(points: Vec<Vec3>) -> Self {
        Self {
            points,
            faces: Vec::new(),
        }
    }

    /// `true` when this carries a topology the backend can trust.
    pub fn is_hulled(&self) -> bool {
        !self.faces.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// The piece scaled; a positive diagonal scale keeps a hull convex with the same faces.
    pub fn scaled(&self, scale: Vec3) -> Self {
        Self {
            points: scaled_points(&self.points, scale),
            faces: self.faces.clone(),
        }
    }
}

/// Collision geometry attached to a body.
#[derive(Debug, Clone, PartialEq)]
pub enum CollisionShape {
    /// Solid ball, parametrised by `radius`.
    Sphere { radius: f32 },
    /// Axis-aligned box in local space. `half_extents` are half the total
    /// side length on each axis.
    Cuboid { half_extents: Vec3 },
    /// Capsule along local Y. `half_height` excludes the hemispherical
    /// caps, so the total length is `2 * (half_height + radius)`.
    Capsule { radius: f32, half_height: f32 },
    /// Cylinder along local Y, flat caps.
    Cylinder { radius: f32, half_height: f32 },
    /// Cylinder with a rounded rim, so wheels and barrels don't snag on box edges.
    RoundCylinder {
        radius: f32,
        half_height: f32,
        border_radius: f32,
    },
    /// Cone along local Y, apex up.
    Cone { radius: f32, half_height: f32 },
    /// Infinite plane through the origin, solid opposite `normal` — a ground without a huge cuboid.
    HalfSpace { normal: Vec3 },
    /// Line between two local-space points. No volume.
    Segment { a: Vec3, b: Vec3 },
    /// Single triangle. No volume.
    Triangle { a: Vec3, b: Vec3, c: Vec3 },
    /// Convex hull of a point cloud: volume, inertia and a cheap narrowphase for dynamic props.
    ConvexHull { part: ConvexPart },
    /// A concave mesh as convex parts, keeping concavities — a bake, not per frame.
    ConvexDecomposition {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
    },
    /// The triangles: right for static level geometry, wrong for dynamic — no volume, no inertia,
    /// ghost edge collisions.
    TriMesh {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
    },
    /// A connected run of segments. No volume.
    Polyline { vertices: Vec<Vec3> },
    /// XZ height grid, column-major `rows × cols`, flat so a jagged grid is unrepresentable.
    Heightfield {
        heights: Vec<f32>,
        rows: u32,
        cols: u32,
        scale: Vec3,
    },
    /// Sparse solid cells, collided directly — smaller than a baked trimesh with no seam ghosts;
    /// what terraforming needs.
    Voxels { size: Vec3, cells: Vec<IVec3> },
    /// Several convex pieces under one collider — how a baked decomposition loads;
    /// [`ConvexDecomposition`](Self::ConvexDecomposition) instead asks VHACD to find them.
    Compound { parts: Vec<ConvexPart> },
    /// A mesh voxelised by the backend at build time — parry already ships it, unlike
    /// [`Voxels`](Self::Voxels).
    VoxelizedMesh {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
        size: f32,
        /// Fill the interior as well as the surface shell.
        solid: bool,
    },
}

impl CollisionShape {
    /// Cells a point cloud occupies, deduplicated and sorted: cell order is observable in the
    /// solver.
    pub fn voxels_from_points(size: Vec3, points: &[Vec3]) -> Self {
        let size = size.max(Vec3::splat(MIN_EXTENT));
        let mut cells: Vec<IVec3> = points
            .iter()
            .map(|point| (*point / size).floor().as_ivec3())
            .collect();
        cells.sort_unstable_by_key(|cell| (cell.x, cell.y, cell.z));
        cells.dedup();
        Self::Voxels { size, cells }
    }

    /// The variant's name, for a message an author can act on.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sphere { .. } => "Sphere",
            Self::Cuboid { .. } => "Cuboid",
            Self::Capsule { .. } => "Capsule",
            Self::Cylinder { .. } => "Cylinder",
            Self::RoundCylinder { .. } => "RoundCylinder",
            Self::Cone { .. } => "Cone",
            Self::HalfSpace { .. } => "HalfSpace",
            Self::Segment { .. } => "Segment",
            Self::Triangle { .. } => "Triangle",
            Self::ConvexHull { .. } => "ConvexHull",
            Self::ConvexDecomposition { .. } => "ConvexDecomposition",
            Self::Compound { .. } => "Compound",
            Self::TriMesh { .. } => "TriMesh",
            Self::Polyline { .. } => "Polyline",
            Self::Heightfield { .. } => "Heightfield",
            Self::Voxels { .. } => "Voxels",
            Self::VoxelizedMesh { .. } => "VoxelizedMesh",
        }
    }

    /// How far the shape extends below its origin on local Y — a controller's ride height must
    /// clear it. `None` for cloud and unbounded shapes.
    pub fn reach(&self) -> Option<f32> {
        match self {
            Self::Sphere { radius } => Some(*radius),
            Self::Cuboid { half_extents } => Some(half_extents.y),
            Self::Capsule {
                radius,
                half_height,
            } => Some(radius + half_height),
            Self::Cylinder { half_height, .. } | Self::Cone { half_height, .. } => {
                Some(*half_height)
            }
            Self::RoundCylinder {
                half_height,
                border_radius,
                ..
            } => Some(half_height + border_radius),
            _ => None,
        }
    }

    /// This shape at a `Transform` scale; rapier shapes take none, so scale rebuilds them. Only
    /// boxes and clouds scale exactly: spheres take the largest axis, Y-aligned shapes the
    /// horizontal ones for radius.
    pub fn scaled(&self, scale: Vec3) -> Self {
        let s = scale.abs();
        let flat = s.x.max(s.z);
        match self {
            Self::Sphere { radius } => Self::Sphere {
                radius: clamp(radius * s.max_element()),
            },
            Self::Cuboid { half_extents } => Self::Cuboid {
                half_extents: (*half_extents * s).max(Vec3::splat(MIN_EXTENT)),
            },
            Self::Capsule {
                radius,
                half_height,
            } => Self::Capsule {
                radius: clamp(radius * flat),
                half_height: clamp(half_height * s.y),
            },
            Self::Cylinder {
                radius,
                half_height,
            } => Self::Cylinder {
                radius: clamp(radius * flat),
                half_height: clamp(half_height * s.y),
            },
            Self::RoundCylinder {
                radius,
                half_height,
                border_radius,
            } => Self::RoundCylinder {
                radius: clamp(radius * flat),
                half_height: clamp(half_height * s.y),
                border_radius: clamp(border_radius * flat),
            },
            Self::Cone {
                radius,
                half_height,
            } => Self::Cone {
                radius: clamp(radius * flat),
                half_height: clamp(half_height * s.y),
            },
            // A normal transforms by the inverse transpose, which for a
            // pure scale is the reciprocal. Scaling it like a point would
            // tilt the ground under a non-uniformly scaled entity.
            Self::HalfSpace { normal } => Self::HalfSpace {
                normal: unit_or_up(*normal / s.max(Vec3::splat(MIN_EXTENT))),
            },
            Self::Segment { a, b } => Self::Segment {
                a: *a * s,
                b: *b * s,
            },
            Self::Triangle { a, b, c } => Self::Triangle {
                a: *a * s,
                b: *b * s,
                c: *c * s,
            },
            Self::ConvexHull { part } => Self::ConvexHull {
                part: part.scaled(s),
            },
            Self::ConvexDecomposition { vertices, indices } => Self::ConvexDecomposition {
                vertices: scaled_points(vertices, s),
                indices: indices.clone(),
            },
            // Each piece scales alone; the scaled union is still a decomposition, so no VHACD
            // rerun.
            Self::Compound { parts } => Self::Compound {
                parts: parts.iter().map(|part| part.scaled(s)).collect(),
            },
            Self::TriMesh { vertices, indices } => Self::TriMesh {
                vertices: scaled_points(vertices, s),
                indices: indices.clone(),
            },
            Self::Polyline { vertices } => Self::Polyline {
                vertices: scaled_points(vertices, s),
            },
            Self::Heightfield {
                heights,
                rows,
                cols,
                scale: extent,
            } => Self::Heightfield {
                heights: heights.clone(),
                rows: *rows,
                cols: *cols,
                scale: *extent * s,
            },
            Self::Voxels { size, cells } => Self::Voxels {
                size: (*size * s).max(Vec3::splat(MIN_EXTENT)),
                cells: cells.clone(),
            },
            // Cell size grows with the largest axis, or cells grow cubically past the trimesh they
            // replace.
            Self::VoxelizedMesh {
                vertices,
                indices,
                size,
                solid,
            } => Self::VoxelizedMesh {
                vertices: scaled_points(vertices, s),
                indices: indices.clone(),
                size: clamp(size * s.max_element()),
                solid: *solid,
            },
        }
    }
}

/// A dimension the solver can build with.
fn clamp(value: f32) -> f32 {
    value.max(MIN_EXTENT)
}

fn scaled_points(points: &[Vec3], scale: Vec3) -> Vec<Vec3> {
    points.iter().map(|point| *point * scale).collect()
}

/// `normal` normalised, or up when zero: a plane with no side cannot be built or seen.
fn unit_or_up(normal: Vec3) -> Vec3 {
    normal.try_normalize().unwrap_or(Vec3::Y)
}

#[cfg(test)]
mod tests;
