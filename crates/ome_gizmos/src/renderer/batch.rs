use glam::Vec3;

use super::types::{DEFAULT_LINE_THICKNESS, LineSegment};

/// Per-frame collection of line segments queued for the gizmo pass.
///
/// Lines are rasterized as **screen-space quads** (sub-phase 3a of
/// #278): each segment becomes a 4-vertex quad whose perpendicular
/// offset is `thickness` physical pixels. Works around `wgpu`'s
/// fixed 1-pixel `LineList` width limitation.
#[derive(Debug, Default)]
pub struct GizmoBatch {
    pub lines: Vec<LineSegment>,
}

impl GizmoBatch {
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Pushes a line segment with [`DEFAULT_LINE_THICKNESS`].
    pub fn line(&mut self, start: Vec3, end: Vec3, color: Vec3) {
        self.line_thick(start, end, color, DEFAULT_LINE_THICKNESS);
    }

    /// Pushes a line segment with explicit screen-space thickness in
    /// physical pixels.
    pub fn line_thick(&mut self, start: Vec3, end: Vec3, color: Vec3, thickness: f32) {
        self.lines.push(LineSegment {
            start,
            end,
            color,
            thickness,
        });
    }

    /// Pushes the 12 edges of an axis-aligned bounding box.
    pub fn aabb(&mut self, min: Vec3, max: Vec3, color: Vec3) {
        let corners = [
            Vec3::new(min.x, min.y, min.z),
            Vec3::new(max.x, min.y, min.z),
            Vec3::new(max.x, max.y, min.z),
            Vec3::new(min.x, max.y, min.z),
            Vec3::new(min.x, min.y, max.z),
            Vec3::new(max.x, min.y, max.z),
            Vec3::new(max.x, max.y, max.z),
            Vec3::new(min.x, max.y, max.z),
        ];
        // Bottom rect (y=min)
        self.line(corners[0], corners[1], color);
        self.line(corners[1], corners[2], color);
        self.line(corners[2], corners[3], color);
        self.line(corners[3], corners[0], color);
        // Top rect (y=max)
        self.line(corners[4], corners[5], color);
        self.line(corners[5], corners[6], color);
        self.line(corners[6], corners[7], color);
        self.line(corners[7], corners[4], color);
        // Vertical pillars
        self.line(corners[0], corners[4], color);
        self.line(corners[1], corners[5], color);
        self.line(corners[2], corners[6], color);
        self.line(corners[3], corners[7], color);
    }

    /// Pushes three world-space axis lines (X red, Y green, Z blue) of
    /// the given length starting at `origin`.
    pub fn axis_lines(&mut self, origin: Vec3, length: f32) {
        const RED: Vec3 = Vec3::new(1.0, 0.25, 0.25);
        const GREEN: Vec3 = Vec3::new(0.25, 1.0, 0.25);
        const BLUE: Vec3 = Vec3::new(0.35, 0.45, 1.0);
        self.line(origin, origin + Vec3::X * length, RED);
        self.line(origin, origin + Vec3::Y * length, GREEN);
        self.line(origin, origin + Vec3::Z * length, BLUE);
    }

    /// Pushes three world-space axis arrows (X red, Y green, Z blue) of
    /// the given length starting at `origin`. Each arrow is the main
    /// shaft plus four small lines forming a 3D arrowhead.
    pub fn axis_arrows(&mut self, origin: Vec3, length: f32) {
        const RED: Vec3 = Vec3::new(1.0, 0.25, 0.25);
        const GREEN: Vec3 = Vec3::new(0.25, 1.0, 0.25);
        const BLUE: Vec3 = Vec3::new(0.35, 0.45, 1.0);
        self.arrow(origin, origin + Vec3::X * length, Vec3::Y, Vec3::Z, RED);
        self.arrow(origin, origin + Vec3::Y * length, Vec3::X, Vec3::Z, GREEN);
        self.arrow(origin, origin + Vec3::Z * length, Vec3::X, Vec3::Y, BLUE);
    }

    /// Pushes a single arrow: shaft + 4 arrowhead segments forming a
    /// "+"-shaped 3D head at `tip`. `perp_a` and `perp_b` are the two
    /// unit-length axes perpendicular to the arrow direction (orient
    /// the arrowhead).
    pub fn arrow(&mut self, base: Vec3, tip: Vec3, perp_a: Vec3, perp_b: Vec3, color: Vec3) {
        let dir = (tip - base).normalize_or_zero();
        let length = (tip - base).length();
        let head_len = (length * 0.15).max(0.05);
        let head_w = head_len * 0.5;
        let back = tip - dir * head_len;
        self.line(base, tip, color);
        self.line(tip, back + perp_a * head_w, color);
        self.line(tip, back - perp_a * head_w, color);
        self.line(tip, back + perp_b * head_w, color);
        self.line(tip, back - perp_b * head_w, color);
    }
}
