//! [`Gizmos`] — borrow-checked accessor over a [`GizmoBatch`].
//!
//! User-facing API for visualizers (and for any system that wants to
//! draw lines in 3D space). Wraps `&mut GizmoBatch` so the underlying
//! storage stays opaque to callers — future render-path changes don't
//! ripple through user code.

use glam::Vec3;

use crate::renderer::GizmoBatch;

/// Borrow-checked handle for pushing line segments to the active
/// [`GizmoBatch`]. Constructed by the gizmo system or test code; users
/// never call `Gizmos::new` directly — they receive a `&mut Gizmos<'_>`
/// inside a [`Visualizer`](crate::Visualizer) implementation.
pub struct Gizmos<'a> {
    batch: &'a mut GizmoBatch,
}

impl<'a> Gizmos<'a> {
    /// Wraps a mutable reference to the batch.
    pub fn new(batch: &'a mut GizmoBatch) -> Self {
        Self { batch }
    }

    /// Pushes a line segment with the default screen-space thickness
    /// ([`crate::DEFAULT_LINE_THICKNESS`]).
    pub fn line(&mut self, start: Vec3, end: Vec3, color: Vec3) {
        self.batch.line(start, end, color);
    }

    /// Pushes a line segment with explicit screen-space thickness in
    /// physical pixels. Use for hover / drag emphasis or any context
    /// where the default 2-pixel default isn't loud enough.
    pub fn line_thick(&mut self, start: Vec3, end: Vec3, color: Vec3, thickness: f32) {
        self.batch.line_thick(start, end, color, thickness);
    }

    /// Pushes the 12 edges of an axis-aligned bounding box.
    pub fn aabb(&mut self, min: Vec3, max: Vec3, color: Vec3) {
        self.batch.aabb(min, max, color);
    }

    /// Pushes three world-space axis lines (X red, Y green, Z blue).
    pub fn axis_lines(&mut self, origin: Vec3, length: f32) {
        self.batch.axis_lines(origin, length);
    }

    /// Pushes three world-space axis arrows (X red, Y green, Z blue)
    /// with 3D `+`-shaped arrowheads at the positive ends.
    pub fn axis_arrows(&mut self, origin: Vec3, length: f32) {
        self.batch.axis_arrows(origin, length);
    }

    /// Pushes a single arrow: main line + 4 arrowhead segments forming
    /// a `+`-shaped 3D head at `tip`. `perp_a` and `perp_b` are unit
    /// vectors perpendicular to the arrow direction.
    pub fn arrow(&mut self, base: Vec3, tip: Vec3, perp_a: Vec3, perp_b: Vec3, color: Vec3) {
        self.batch.arrow(base, tip, perp_a, perp_b, color);
    }
}
