use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Default screen-space thickness in physical pixels for `line` calls.
pub const DEFAULT_LINE_THICKNESS: f32 = 2.0;

/// One line segment to be drawn in world space, plus its rendered
/// thickness in physical pixels.
#[derive(Debug, Clone, Copy)]
pub struct LineSegment {
    pub start: Vec3,
    pub end: Vec3,
    pub color: Vec3,
    pub thickness: f32,
}

// ---------------------------------------------------------------------------
// GPU types — vertex format + camera uniforms
// ---------------------------------------------------------------------------

/// Quad-line vertex, six per line. `other_position` is the far endpoint for the perpendicular;
/// `side` is ±1; `thickness` is in physical pixels.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(super) struct GizmoVertex {
    pub(super) position: [f32; 3],
    pub(super) color: [f32; 3],
    pub(super) other_position: [f32; 3],
    pub(super) side: f32,
    pub(super) thickness: f32,
}

/// Matches `CameraUniforms` in `gizmo_main.wgsl`; `viewport_size` turns pixel thickness into NDC
/// offsets.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub(super) struct CameraUniforms {
    pub(super) view_proj: [[f32; 4]; 4],
    pub(super) viewport_size: [f32; 2],
    pub(super) _pad: [f32; 2],
}

/// Initial vertex buffer capacity in vertices (= 6 × line capacity).
/// Grows on demand if the batch overflows.
pub(super) const INITIAL_VERTEX_CAPACITY: u64 = 4096;
