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

/// Quad-line vertex. Each line emits 6 vertices (two triangles).
///
/// `position` is this vertex's 3D world endpoint; `other_position` is
/// the line's other endpoint (used by the vertex shader to compute the
/// perpendicular direction). `side` is `+1` or `-1` indicating which
/// side of the line this vertex sits on. `thickness` is in physical
/// pixels and controls the perpendicular offset magnitude.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(super) struct GizmoVertex {
    pub(super) position: [f32; 3],
    pub(super) color: [f32; 3],
    pub(super) other_position: [f32; 3],
    pub(super) side: f32,
    pub(super) thickness: f32,
}

/// Matches `CameraUniforms` in `gizmo_main.wgsl`.
///
/// `view_proj` projects world points to clip space; `viewport_size`
/// (physical pixels) lets the shader convert pixel-thickness into
/// NDC offsets.
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
