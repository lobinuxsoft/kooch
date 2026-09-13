//! Sub-pixel camera jitter — the other half of a temporal resolve (#481).

use glam::{Mat4, Vec2, Vec3};

/// How many offsets before the sequence repeats, at 1:1.
pub const JITTER_BASE_PHASES: u32 = 8;

/// The count at 3× upscaling, which is FSR's most aggressive preset
/// (Ultra Performance) and therefore the largest ratio anything here
/// will legitimately ask for.
pub const JITTER_MAX_PHASES: u32 = JITTER_BASE_PHASES * 9;

/// How many phases to run when rendering at `render_width` and presenting at `display_width`.
pub fn phase_count(render_width: u32, display_width: u32) -> u32 {
    let ratio = display_width.max(1) as f32 / render_width.max(1) as f32;
    let scaled = (JITTER_BASE_PHASES as f32 * ratio * ratio).ceil() as u32;
    scaled.clamp(JITTER_BASE_PHASES, JITTER_MAX_PHASES)
}

/// One frame's sub-pixel offset and the matrices that follow from it.
#[derive(Debug, Clone, Copy)]
pub struct Jitter {
    /// Offset in PIXELS, in `[-0.5, 0.5]` per axis. Zero when temporal
    /// anti-aliasing is off.
    pub pixels: Vec2,
    /// What the raster and every reconstruction off its visibility
    /// buffer must use.
    pub view_proj: Mat4,
    /// What the motion vectors must use. The camera's own matrix,
    /// untouched.
    pub unjittered: Mat4,
}

impl Jitter {
    /// The identity: no offset, both matrices the same.
    pub fn none(view_proj: Mat4) -> Self {
        Self {
            pixels: Vec2::ZERO,
            view_proj,
            unjittered: view_proj,
        }
    }

    /// Frame `index`'s offset applied to `view_proj`.
    pub fn at(index: u32, view_proj: Mat4, size: (u32, u32), phases: u32) -> Self {
        let pixels = offset(index, phases);
        let width = size.0.max(1) as f32;
        let height = size.1.max(1) as f32;
        // NDC spans 2 across the viewport, hence the doubling; V runs
        // down where NDC Y runs up, hence the flip.
        let ndc = Vec3::new(2.0 * pixels.x / width, -2.0 * pixels.y / height, 0.0);
        Self {
            pixels,
            // Pre-multiplied, so the offset is added to clip x/y in proportion to w and survives
            // the perspective divide as a constant NDC shift. Post-multiplying would offset the
            // camera in WORLD space instead, which is parallax, not anti-aliasing.
            view_proj: Mat4::from_translation(ndc) * view_proj,
            unjittered: view_proj,
        }
    }
}

/// Frame `index`'s offset in pixels, in `[-0.5, 0.5]`.
fn offset(index: u32, phases: u32) -> Vec2 {
    let n = index % phases.max(1) + 1;
    Vec2::new(radical_inverse(n, 2) - 0.5, radical_inverse(n, 3) - 0.5)
}

/// The van der Corput radical inverse: `index` written in `base`, then
/// mirrored about the decimal point.
fn radical_inverse(mut index: u32, base: u32) -> f32 {
    let mut result = 0.0f32;
    let mut fraction = 1.0f32 / base as f32;
    while index > 0 {
        result += fraction * (index % base) as f32;
        index /= base;
        fraction /= base as f32;
    }
    result
}

#[cfg(test)]
mod tests;
