//! Snapdragon Game Super Resolution 2, transliterated (#481, step 4).
//!
//! The engine's own temporal upscaler, ported rather than invented. The
//! reasoning behind picking this one over FSR 3.1 is in the header of
//! `sgsr2_convert.wgsl`; the short version is that FSR has the quality
//! and SGSR has the cost, and cost is what this engine is short of —
//! 40.7 ms against a 13.9 ms budget.
//!
//! # 🎯 It has an oracle, which the plan said this work would not
//!
//! The risk written into #481 was that a transliteration has nothing to
//! diff against, so it is validated by eye and degenerates into "it
//! looks wrong and I do not know why".
//!
//! That is avoidable: **at a ratio of 1:1 this IS a TAA**. Run it
//! un-upscaled against the resolve that already ships, on the same
//! frames, and a port that is wrong shows up as a difference from a
//! known-good image rather than as a vague softness. It separates "did I
//! port it correctly" from "does the resolution split work", which are
//! the two risks the plan had bundled into one.
//!
//! # Licence
//!
//! BSD 3-Clause, Qualcomm Innovation Center. The copyright header stays
//! in every ported file and the full text is in `NOTICE`. The third
//! clause also forbids using their name to endorse this — so: **this is
//! not a Qualcomm product and Qualcomm has not endorsed it.**

// ⚠️ Unused until the upscale pass lands, and allowed rather than left
// to warn: these constants are compiled into every project that builds
// on this engine, and a warning in someone else's build is noise they
// cannot act on. The `#[allow]` comes off with the pass that uses them.
#![allow(dead_code)]

const CONVERT_SOURCE: &str = include_str!("../../../shaders/sgsr2_convert.wgsl");

/// What the convert pass writes and the upscale pass reads.
///
/// `xy` dilated motion in UV, `z` the depth-clip factor, `w` unused.
/// Half precision: the motion is already `Rg16Float` upstream of this
/// and the clip factor is a `[0, 1]` weight, so nothing here has a range
/// that fp16 cannot describe.
pub const SGSR2_CONVERT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Upstream's `Params.cameraFovAngleHor`.
///
/// 🔴 Verified rather than guessed, which matters because it scales a
/// tuned constant. Qualcomm's public repository ships **only the
/// shaders** — no host code — so the value was recovered from a
/// community Unity port (`whitecostume/SGSR2_Unity`), which computes
/// `tan(fov_vertical / 2) * aspect`. That is `tan(fov_horizontal / 2)`,
/// and it agrees with the dimensional analysis of the expression it
/// feeds: two independent routes to the same number.
pub fn fov_k(fov_vertical: f32, aspect: f32) -> f32 {
    (fov_vertical * 0.5).tan() * aspect
}

/// Upstream's `Params.scaleRatio`.
///
/// `.x` is the linear upscale ratio and becomes the Lanczos kernel's
/// bias. `.y` is the **cube of the area ratio, capped at 20** — their
/// number, and the cap is theirs too. It widens the variance box as the
/// upscale gets more aggressive, because a box built from fewer input
/// samples is a worse estimate of the neighbourhood and clamping the
/// history to it too tightly is what makes an upscaler flicker.
///
/// At 1:1 it is `(1, 1)`, which is the identity this is validated at.
pub fn scale_ratio(render: (u32, u32), display: (u32, u32)) -> [f32; 2] {
    let linear = display.0.max(1) as f32 / render.0.max(1) as f32;
    let area = (display.0.max(1) as f32 * display.1.max(1) as f32)
        / (render.0.max(1) as f32 * render.1.max(1) as f32);
    [linear, (area * area * area).min(20.0)]
}

/// Upstream's `Params.minLerpContribution`, unchanged at their default.
///
/// How much of the history survives when it lands outside the
/// neighbourhood box **and the pixel is not moving**. A moving pixel
/// gets zero instead — see the upscale shader.
pub const MIN_LERP_CONTRIBUTION: f32 = 0.3;

#[cfg(test)]
mod tests;
