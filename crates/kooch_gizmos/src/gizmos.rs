//! [`Gizmos`] — borrow-checked accessor over the line and mesh batches.
//!
//! User-facing API for visualizers and any system that wants to draw
//! gizmo geometry. Wraps `&mut GizmoBatch` (lines) and `&mut MeshBatch`
//! (filled triangles) so the underlying storage stays opaque to
//! callers — future render-path changes don't ripple through user code.

use glam::{Mat3, Vec3, Vec4};

use crate::mesh::MeshBatch;
use crate::renderer::GizmoBatch;

/// Borrow-checked handle for pushing gizmo geometry. Constructed by
/// the editor's gizmo system; users receive a `&mut Gizmos<'_>` inside
/// a [`Visualizer`](crate::Visualizer) implementation.
pub struct Gizmos<'a> {
    line_batch: &'a mut GizmoBatch,
    mesh_batch: &'a mut MeshBatch,
}

impl<'a> Gizmos<'a> {
    pub fn new(line_batch: &'a mut GizmoBatch, mesh_batch: &'a mut MeshBatch) -> Self {
        Self {
            line_batch,
            mesh_batch,
        }
    }

    // -----------------------------------------------------------------
    // Line primitives (forward to GizmoBatch)
    // -----------------------------------------------------------------

    /// Pushes a line segment with the default screen-space thickness
    /// ([`crate::DEFAULT_LINE_THICKNESS`]).
    pub fn line(&mut self, start: Vec3, end: Vec3, color: Vec3) {
        self.line_batch.line(start, end, color);
    }

    /// Pushes a line segment with explicit screen-space thickness in
    /// physical pixels.
    pub fn line_thick(&mut self, start: Vec3, end: Vec3, color: Vec3, thickness: f32) {
        self.line_batch.line_thick(start, end, color, thickness);
    }

    /// Pushes the 12 edges of an axis-aligned bounding box.
    pub fn aabb(&mut self, min: Vec3, max: Vec3, color: Vec3) {
        self.line_batch.aabb(min, max, color);
    }

    /// Pushes three world-space axis lines (X red, Y green, Z blue).
    pub fn axis_lines(&mut self, origin: Vec3, length: f32) {
        self.line_batch.axis_lines(origin, length);
    }

    /// Pushes three world-space axis arrows (X red, Y green, Z blue)
    /// with 3D `+`-shaped arrowheads at the positive ends.
    pub fn axis_arrows(&mut self, origin: Vec3, length: f32) {
        self.line_batch.axis_arrows(origin, length);
    }

    /// Pushes a single arrow: shaft + 4 arrowhead segments forming a
    /// `+`-shaped 3D head at `tip`.
    pub fn arrow(&mut self, base: Vec3, tip: Vec3, perp_a: Vec3, perp_b: Vec3, color: Vec3) {
        self.line_batch.arrow(base, tip, perp_a, perp_b, color);
    }

    // -----------------------------------------------------------------
    // Mesh primitives (forward to MeshBatch)
    // -----------------------------------------------------------------

    /// Pushes a filled, alpha-blended quad from four corners (CCW order).
    /// Color is RGBA — the alpha component controls transparency.
    pub fn filled_quad(&mut self, p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, color: Vec4) {
        self.mesh_batch.filled_quad(p0, p1, p2, p3, color);
    }

    /// Pushes a filled axis-aligned box (12 triangles), centered at
    /// `center` with the given world-space half-extents.
    pub fn filled_aabb(&mut self, center: Vec3, half_extents: Vec3, color: Vec4) {
        self.mesh_batch.filled_aabb(center, half_extents, color);
    }

    /// Pushes a filled oriented box — like [`Self::filled_aabb`] but
    /// the faces are rotated by `basis` (each column is a face axis).
    pub fn filled_obb(&mut self, center: Vec3, basis: Mat3, half_extents: Vec3, color: Vec4) {
        self.mesh_batch.filled_obb(center, basis, half_extents, color);
    }

    /// Pushes a filled 3D arrow from `base` to `tip` — an octagonal
    /// cylinder shaft topped with an octagonal cone head. Unlike the
    /// line-based [`Self::arrow`], this is a solid mesh — used by the
    /// translate handle.
    pub fn filled_arrow(&mut self, base: Vec3, tip: Vec3, color: Vec4) {
        self.mesh_batch.filled_arrow(base, tip, color);
    }

    /// Pushes a filled torus around `axis` through `center`. Used by
    /// the rotate handle.
    pub fn filled_torus(
        &mut self,
        center: Vec3,
        axis: Vec3,
        major_radius: f32,
        minor_radius: f32,
        color: Vec4,
    ) {
        self.mesh_batch
            .filled_torus(center, axis, major_radius, minor_radius, color);
    }
}
