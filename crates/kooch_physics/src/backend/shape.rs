//! [`CollisionShape`] — the geometry a body presents to the solver.
//!
//! # Plain glam, never a backend type
//!
//! Every variant carries `f32`, `Vec3`, `IVec3` or a `Vec` of those. No
//! `SharedShape`, no `TriMeshFlags`, and — the one worth stating — **no
//! asset handle**. A mesh-derived shape carries its points, because a
//! `Guid` here would drag the renderer in behind it and tie the physics
//! trait to wgpu. The bridge runs the other way: [`ColliderMeshCache`]
//! is defined here and filled by whoever can already see meshes.
//!
//! [`ColliderMeshCache`]: super::ColliderMeshCache
//!
//! # Why this is not `Copy`
//!
//! It was, while the vocabulary was three primitives. A convex hull is a
//! point cloud, and a level's trimesh is hundreds of thousands of
//! triangles — copying either on a whim is not something a per-frame path
//! should be able to do by accident. The sync pass builds one per body
//! per *rebuild*, not per frame; see [`ShapeSpec`](crate::components::ShapeSpec)
//! for the cheap POD identity it compares instead.

use glam::{IVec3, Vec3};

/// Smallest dimension a shape is built with.
///
/// A field mid-edit in the Inspector passes through zero on the way to
/// the value the author means, and a zero-radius shape makes the solver
/// produce NaNs that outlive the typo.
pub const MIN_EXTENT: f32 = 1e-4;

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
    /// Cylinder with its rim rounded off by `border_radius`.
    ///
    /// The cheap fillet that stops a wheel or a barrel snagging on a box
    /// edge — a sharp rim gives the solver a single contact point to
    /// resolve, and it catches.
    RoundCylinder {
        radius: f32,
        half_height: f32,
        border_radius: f32,
    },
    /// Cone along local Y, apex up.
    Cone { radius: f32, half_height: f32 },
    /// Infinite plane through the body's origin, solid on the side
    /// `normal` points away from.
    ///
    /// The one-line ground: a test scene stops needing a cuboid big
    /// enough to never be walked off, which is a shape whose only job was
    /// to be large.
    HalfSpace { normal: Vec3 },
    /// Line between two local-space points. No volume.
    Segment { a: Vec3, b: Vec3 },
    /// Single triangle. No volume.
    Triangle { a: Vec3, b: Vec3, c: Vec3 },
    /// The convex hull of a point cloud.
    ///
    /// The standard answer for a dynamic prop whose visual mesh is too
    /// heavy to collide against: convex, so it has volume and an inertia
    /// tensor, and cheap for the narrowphase.
    ConvexHull { points: Vec<Vec3> },
    /// A concave mesh approximated by a set of convex parts.
    ///
    /// What a single hull cannot do: keep a concavity a designer is
    /// relying on. Expensive to build — this is a bake, not a per-frame
    /// operation.
    ConvexDecomposition {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
    },
    /// The triangles themselves.
    ///
    /// Correct for static level geometry and wrong for anything dynamic:
    /// no volume, no inertia, and ghost collisions where a body slides
    /// across a shared edge.
    TriMesh {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
    },
    /// A connected run of segments. No volume.
    Polyline { vertices: Vec<Vec3> },
    /// A height grid on the XZ plane, column-major, `rows × cols`.
    ///
    /// Flat plus its dimensions rather than a nested `Vec`, because that
    /// is the layout the backend wants and a jagged grid should be
    /// unrepresentable.
    Heightfield {
        heights: Vec<f32>,
        rows: u32,
        cols: u32,
        scale: Vec3,
    },
    /// A sparse grid of solid cells.
    ///
    /// Rapier is the only general-purpose solver shipping this, and it
    /// matters here more than elsewhere: it collides against the voxels
    /// directly, so it is smaller than a baked trimesh and has no seam
    /// ghost-collisions. The shape terraforming needs.
    Voxels { size: Vec3, cells: Vec<IVec3> },
    /// A mesh the backend voxelises at build time.
    ///
    /// Separate from [`Voxels`](Self::Voxels) because the voxelisation is
    /// the backend's — asking the engine to rasterise a mesh into cells
    /// would be reimplementing what parry already ships.
    VoxelizedMesh {
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
        size: f32,
        /// Fill the interior as well as the surface shell.
        solid: bool,
    },
}

impl CollisionShape {
    /// The cells a point cloud occupies on a grid of `size`.
    ///
    /// Deduplicated and sorted, so the same cloud always produces the
    /// same shape — cell order is observable in the solver.
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
            Self::TriMesh { .. } => "TriMesh",
            Self::Polyline { .. } => "Polyline",
            Self::Heightfield { .. } => "Heightfield",
            Self::Voxels { .. } => "Voxels",
            Self::VoxelizedMesh { .. } => "VoxelizedMesh",
        }
    }

    /// Whether the solver gets a closed volume, and therefore an inertia
    /// tensor worth having.
    ///
    /// A dynamic body on a hollow shape tumbles wrongly and tunnels
    /// through its own edges, so this is what a warning keys on.
    pub fn is_solid(&self) -> bool {
        !matches!(
            self,
            Self::Segment { .. }
                | Self::Triangle { .. }
                | Self::TriMesh { .. }
                | Self::Polyline { .. }
                | Self::Heightfield { .. }
                | Self::HalfSpace { .. }
        )
    }

    /// This shape at a `Transform` scale.
    ///
    /// Rapier's shapes take no scale — they are built from dimensions —
    /// so scaling happens where the shape is built, and a scale change
    /// rebuilds it.
    ///
    /// # Why the round shapes are approximations
    ///
    /// Only a box and a point cloud scale exactly. A non-uniformly scaled
    /// sphere is an ellipsoid and rapier has no ellipsoid, so the round
    /// shapes follow the convention every engine uses: a sphere takes the
    /// largest axis, because a collider smaller than what you can see is
    /// the one that reads as a physics bug; the Y-aligned shapes take
    /// their radius from the horizontal axes, so scaling on Y makes them
    /// taller rather than fatter.
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
            Self::ConvexHull { points } => Self::ConvexHull {
                points: scaled_points(points, s),
            },
            Self::ConvexDecomposition { vertices, indices } => Self::ConvexDecomposition {
                vertices: scaled_points(vertices, s),
                indices: indices.clone(),
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
            // The cell size grows with the largest axis: a finer grid over
            // scaled-up geometry costs cells cubically, and the voxel
            // shape's whole reason to exist is being cheaper than the
            // trimesh it came from.
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

/// `normal`, normalised, or up when it has no direction to give.
///
/// A zero normal is a plane with no side, which rapier cannot build and
/// the author cannot see. Falling back to a floor is the recoverable
/// reading of a half-authored field.
fn unit_or_up(normal: Vec3) -> Vec3 {
    normal.try_normalize().unwrap_or(Vec3::Y)
}

#[cfg(test)]
mod tests;
