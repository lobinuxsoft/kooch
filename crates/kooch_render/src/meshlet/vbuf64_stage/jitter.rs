//! Sub-pixel camera jitter — the other half of a temporal resolve (#481).
//!
//! Motion vectors say where a surface was. Jitter is what makes asking
//! worth anything: without it every frame samples the same point at the
//! centre of every pixel, the history is a copy of the present, and TAA
//! blends an image with itself. The signal a temporal resolve integrates
//! is the *difference* between frames, and on a still camera in a still
//! scene there is none unless the projection puts it there.
//!
//! # Halton(2, 3), and why not random
//!
//! A random offset per frame clusters — eight random points in a pixel
//! leave holes and pile up, so the accumulated image is unevenly
//! sampled and the unevenness moves. A low-discrepancy sequence fills
//! the pixel by construction: every prefix of it is spread out, so the
//! partial sum after three frames is already close to uniform, which is
//! what a resolve that keeps rejecting its history actually gets.
//!
//! Halton(2, 3) is what UE, Unity, Bevy and FSR all use. How many of
//! its terms to take before repeating is the part that differs, and it
//! is not a taste question — see [`phase_count`].
//!
//! # 🔴 The jitter is a lie told to exactly one matrix
//!
//! It goes into the projection the raster uses, and therefore into the
//! barycentric reconstruction that reads the same visibility buffer —
//! those two must agree or the surface point is wrong. It must NOT go
//! into the pair of matrices the motion vectors are computed from: a
//! vector carrying the jitter would describe it as scene motion, and the
//! reprojection would cancel exactly the signal TAA exists to integrate.
//!
//! Which is why this returns both matrices rather than replacing one.

use glam::{Mat4, Vec2, Vec3};

/// How many offsets before the sequence repeats, at 1:1.
///
/// Eight, which is FSR's base and the industry figure.
///
/// 🔴 It shipped at SIXTEEN for one session and was reverted, and the
/// reasoning that put it there is worth keeping because it sounded
/// right: the resolve's minimum blend rate is 1/64, so a pixel that
/// never rejects its history integrates over sixty-odd frames, and
/// eight distinct points leave it averaging each one seven times over
/// instead of covering more of the pixel.
///
/// What that argument ignores is the pixel that DOES reject. Confidence
/// resets the moment a pixel moves, so a moving camera blends at 0.1
/// and never gets near the floor — and there the sequence is not being
/// integrated, it is being SEEN. Sixteen positions swung twice as far
/// as eight before the resolve could average them, and the owner
/// reported it as violent jitter.
///
/// ⚠️ The general shape of the mistake: a constant justified by the
/// converged case and paid for by the transient one. This resolve
/// spends most of its life in the transient.
pub const JITTER_BASE_PHASES: u32 = 8;

/// The count at 3× upscaling, which is FSR's most aggressive preset
/// (Ultra Performance) and therefore the largest ratio anything here
/// will legitimately ask for.
pub const JITTER_MAX_PHASES: u32 = JITTER_BASE_PHASES * 9;

/// How many phases to run when rendering at `render_width` and
/// presenting at `display_width`.
///
/// FSR's rule, with our base: the count scales with the **square** of
/// the ratio, because what the sequence has to cover is an area. At 1.5×
/// upscaling every output pixel is fed by 1/2.25 of an input one, so
/// reaching the same sub-pixel density needs 2.25× the samples — take
/// the ratio linearly and the reconstruction is starved exactly where
/// upscaling is doing the most work.
///
/// Never returns less than [`JITTER_BASE_PHASES`]: rendering above
/// display resolution is supersampling, which needs no help from the
/// projection, and shrinking the sequence there would only shorten the
/// history for no gain.
///
/// 🔴 And never more than [`JITTER_MAX_PHASES`]. The ceiling is not
/// defensive tidying: a render width of zero — a minimised window, a
/// target queried a frame early — sends the square to tens of millions,
/// and a period that large is a sequence that *never repeats*. The
/// accumulated image would then be a drifting sample set instead of a
/// fixed one, which is soft in a way no capture explains.
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
    ///
    /// `size` is the viewport in pixels, because the offset is expressed
    /// in pixels and clip space is not: a half-pixel is one NDC unit
    /// divided by half the width, and getting that wrong scales the
    /// jitter with the window.
    ///
    /// 🔴 When the resolution split lands, `size` is the **render**
    /// target, not the presented one. The camera is jittered by a
    /// fraction of the pixel it actually rasterises; measuring it
    /// against the output would shrink every offset by the ratio and
    /// leave the sequence covering a fraction of the pixel it is
    /// supposed to fill.
    ///
    /// `phases` comes from [`phase_count`].
    pub fn at(index: u32, view_proj: Mat4, size: (u32, u32), phases: u32) -> Self {
        let pixels = offset(index, phases);
        let width = size.0.max(1) as f32;
        let height = size.1.max(1) as f32;
        // NDC spans 2 across the viewport, hence the doubling; V runs
        // down where NDC Y runs up, hence the flip.
        let ndc = Vec3::new(2.0 * pixels.x / width, -2.0 * pixels.y / height, 0.0);
        Self {
            pixels,
            // Pre-multiplied, so the offset is added to clip x/y in
            // proportion to w and survives the perspective divide as a
            // constant NDC shift. Post-multiplying would offset the
            // camera in WORLD space instead, which is parallax, not
            // anti-aliasing.
            view_proj: Mat4::from_translation(ndc) * view_proj,
            unjittered: view_proj,
        }
    }
}

/// Frame `index`'s offset in pixels, in `[-0.5, 0.5]`.
///
/// The sequence is 1-based: Halton's first term is 0 in every base, and
/// an offset of exactly zero is the one frame that contributes nothing
/// new.
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
