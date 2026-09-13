//! What the frame costs, as the author set it (#830).

use crate::meshlet::ShadingRate;

/// Which technique accumulates frames (#481, #536).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpscaleTechnique {
    /// No accumulation and no jitter. What every capture before #481 was taken against, and still
    /// the default: a temporal resolve rewrites every pixel of the image, which is not something to
    /// adopt on behalf of a project that never asked for it.
    #[default]
    None,
    /// The engine's own resolve — the Playdead / Karis lineage, by way
    /// of Bevy, with the departures recorded in `taa.wgsl`. Resolves at
    /// render resolution; it antialiases and does not upscale.
    Taa,
    /// Snapdragon Game Super Resolution 2, transliterated (BSD-3).
    /// Resolves **and** upscales. Two passes, and the cheap one.
    Sgsr2,
    /// AMD FidelityFX Super Resolution 3.1, transliterated (MIT). Resolves **and** upscales. Six
    /// dispatches against SGSR 2's two, and what they buy is feature locking, reactivity and an
    /// exact disocclusion test — the things that stop an upscaler reading as soft.
    Fsr3,
    /// NVIDIA DLSS Super Resolution, through `dlss_wgpu` (#536).
    Dlss,
}

impl UpscaleTechnique {
    /// Whether anything accumulates history, and therefore whether the camera jitters and the
    /// motion vectors are written.
    pub fn is_temporal(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Whether the technique renders at a lower resolution than it
    /// presents. Distinct from [`Self::is_temporal`]: a resolve that
    /// only antialiases is temporal and not upscaling.
    pub fn upscales(self) -> bool {
        matches!(self, Self::Sgsr2 | Self::Fsr3 | Self::Dlss)
    }

    /// The value as it is written in a `.rendersettings` file.
    pub fn from_asset(value: u32) -> Self {
        match value {
            1 => Self::Taa,
            2 => Self::Sgsr2,
            3 => Self::Fsr3,
            4 => Self::Dlss,
            _ => Self::None,
        }
    }
}

/// Which technique accumulates frames, as the project asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalSettings {
    pub technique: UpscaleTechnique,
    /// Render width as a percentage of the output's, 1..=100.
    pub render_scale: u32,
    /// How hard RCAS sharpens the finished image, 0..=100 (#481 step 5).
    pub sharpening: u32,
}

impl UpscaleTechnique {
    /// The render target's size for an output of `output`.
    pub fn render_size(self, output: (u32, u32), scale: u32) -> (u32, u32) {
        if !self.upscales() || scale >= 100 {
            return output;
        }
        let s = scale.clamp(1, 100) as f32 / 100.0;
        (
            ((output.0 as f32 * s) as u32).max(1),
            ((output.1 as f32 * s) as u32).max(1),
        )
    }
}

impl Default for TemporalSettings {
    /// Off, unless the environment says otherwise.
    fn default() -> Self {
        Self::new(UpscaleTechnique::None, 100, 0, false)
    }
}

/// Which shading path runs and how much of the frame it evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadingSettings {
    /// Compute shading (#824) rather than the fragment path.
    pub compute: bool,
    /// Pixels per shaded sample (#825). [`ShadingRate::Half`] requires
    /// `compute`; the stage refuses it otherwise rather than half
    /// applying it.
    pub rate: ShadingRate,
    /// Samples the texture filter takes along the long axis of a footprint, 1..=16 (1 = off).
    pub anisotropy: u16,
}

impl Default for ShadingSettings {
    /// What the process environment says, so a measurement run that sets
    /// nothing else still behaves as it did before this module existed.
    fn default() -> Self {
        Self {
            compute: crate::meshlet::compute_shading_override().unwrap_or(false),
            rate: crate::meshlet::shading_rate_override().unwrap_or_default(),
            anisotropy: 1,
        }
    }
}

impl ShadingSettings {
    /// The author's values with any `KOOCH_*` override applied on top.
    pub fn from_asset(compute: bool, rate: ShadingRate, anisotropy: u16) -> Self {
        Self {
            compute: crate::meshlet::compute_shading_override().unwrap_or(compute),
            rate: crate::meshlet::shading_rate_override().unwrap_or(rate),
            // Clamped to what hardware implements. A driver rounds an
            // in-between value down anyway, and 0 is not a legal
            // sampler.
            anisotropy: anisotropy.clamp(1, 16),
        }
    }
}

/// How finished frames reach the display, as the project asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presentation {
    /// Whether the surface waits for the vblank.
    pub vsync: bool,
}

impl Presentation {
    /// The asset's choice, with `KOOCH_PRESENT_MODE` on top.
    pub fn from_asset(vsync: bool) -> Self {
        Self::resolve(vsync, kooch_core::gpu::vsync_override())
    }

    /// The precedence rule, apart from the read, so a test can exercise it without touching the
    /// process environment.
    fn resolve(asset: bool, over: Option<bool>) -> Self {
        Self {
            vsync: over.unwrap_or(asset),
        }
    }
}

impl Default for Presentation {
    fn default() -> Self {
        Self::from_asset(true)
    }
}

impl TemporalSettings {
    /// `compute` is whether the compute shading path is on, and it is a gate on the scale for the
    /// same reason the technique is.
    pub fn new(
        technique: UpscaleTechnique,
        render_scale: u32,
        sharpening: u32,
        compute: bool,
    ) -> Self {
        Self {
            render_scale: if technique.upscales() && compute {
                render_scale
            } else {
                100
            },
            // Clamped once, here, for the same reason the scale is gated once: every consumer
            // downstream then gets a value it can use without asking whether someone typed 500 into
            // a text file.
            sharpening: sharpening_override().unwrap_or(sharpening).min(100),
            // 🔴 The variable is still a BOOLEAN, and deliberately so. It exists to force a
            // technique on or off from a Steam launch option while capturing on the handheld, where
            // the question is "what does this cost", not "which of three".
            technique: match temporal_aa_override() {
                Some(true) if technique.is_temporal() => technique,
                Some(true) => UpscaleTechnique::Taa,
                Some(false) => UpscaleTechnique::None,
                None => technique,
            },
        }
    }

    /// Whether a history is accumulated at all.
    pub fn enabled(&self) -> bool {
        self.technique.is_temporal()
    }
}

/// `KOOCH_TEMPORAL_AA=on` (or `off`), read once.
pub fn temporal_aa_override() -> Option<bool> {
    static ON: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    *ON.get_or_init(
        || match std::env::var("KOOCH_TEMPORAL_AA").ok().as_deref() {
            Some("on") | Some("1") | Some("true") => {
                tracing::info!(
                    target: "kooch_render::quality",
                    "KOOCH_TEMPORAL_AA=on: the camera jitters by a sub-pixel Halton \
                     offset and each frame is blended with the reprojected one before \
                     it",
                );
                Some(true)
            }
            Some("off") | Some("0") | Some("false") => Some(false),
            _ => None,
        },
    )
}

/// `KOOCH_SHARPENING=0..100`, read once.
pub fn sharpening_override() -> Option<u32> {
    static AMOUNT: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *AMOUNT.get_or_init(|| {
        let raw = std::env::var("KOOCH_SHARPENING").ok()?;
        let percent = raw.trim().parse::<u32>().ok()?.min(100);
        tracing::info!(
            target: "kooch_render::quality",
            percent,
            "KOOCH_SHARPENING: the finished image is sharpened by RCAS",
        );
        Some(percent)
    })
}

#[cfg(test)]
mod tests;
