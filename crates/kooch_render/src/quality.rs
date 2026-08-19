//! What the frame costs, as the author set it (#830).
//!
//! Exposure and shadow distance describe how a project **looks**, and
//! they live in [`RenderSettings`](crate::settings::RenderSettings)
//! beside each other. These describe how much the renderer is willing to
//! **spend** getting there: which shading path runs, at what rate, how
//! many of a froxel's lights each pixel evaluates, whether frames are
//! accumulated.
//!
//! # Why they were environment variables, and still are
//!
//! Every one of these arrived as `KOOCH_*` because the editor is not
//! where they can be measured — the frame they exist for is a game on a
//! handheld launched through Steam, with no editor in the process. That
//! has not changed and the variables stay.
//!
//! What changed is that a shipped game had no way to set them at all: a
//! build ran whatever the constructor's default was, forever. So the
//! order is **environment first, asset second**:
//!
//! - the `.rendersettings` asset says what the project ships with;
//! - a `KOOCH_*` variable, when present, overrides it for that run.
//!
//! A measurement run therefore still gets exactly what it asked for, and
//! a player launching the game gets what the author chose. Inverting the
//! two would make an A/B run depend on which project happened to be
//! open, which is how a capture ends up measuring the wrong thing.
//!
//! # Why absent means "no opinion"
//!
//! [`MeshletRenderStage::render`](crate::meshlet::MeshletRenderStage)
//! applies these only when the resource is actually present. A test that
//! calls `set_shading_rate` directly and then renders must not have its
//! choice overwritten by a default nobody asked for, and a project with
//! no settings asset must render exactly as it did before this module
//! existed.

use crate::meshlet::ShadingRate;

/// Which technique accumulates frames (#481, #536).
///
/// # Strategy, dispatched by enum rather than by trait object
///
/// The techniques are interchangeable behaviours behind one contract,
/// which is Strategy — but not `Box<dyn Upscaler>`. The set is **closed
/// by construction**: an engine ships the techniques it ships, and
/// nothing downstream can define a new one. That turns the usual
/// trade-off inside out — an enum costs no allocation, no vtable and no
/// pointer chase, the compiler checks that every site handles every
/// variant, and the one `match` per frame is not a hot path in any
/// meaningful sense.
///
/// This is also what the project's Rust rules require: identities are
/// values, not pointers.
///
/// # What each variant owes
///
/// Every technique consumes the same six inputs — jittered colour,
/// depth, motion vectors, the jitter offset, exposure and a reset flag
/// — and returns a resolved image. That contract is FSR 3.1's, kept
/// deliberately so a third-party backend can be added later without
/// changing what the renderer hands it.
///
/// 🔴 **Jitter and resolve are not separable.** The sub-pixel jitter is
/// what gives a resolve something to integrate, and the resolve is what
/// makes the jitter anything other than a wobble. Half of the pair is
/// always worse than neither, which is why this is one setting and not
/// two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpscaleTechnique {
    /// No accumulation and no jitter. What every capture before #481
    /// was taken against, and still the default: a temporal resolve
    /// rewrites every pixel of the image, which is not something to
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
    /// AMD FidelityFX Super Resolution 3.1, transliterated (MIT).
    /// Resolves **and** upscales. Six dispatches against SGSR 2's two,
    /// and what they buy is feature locking, reactivity and an exact
    /// disocclusion test — the things that stop an upscaler reading as
    /// soft.
    ///
    /// 🔴 **Measured, and it does not fit a handheld.** 11.682 ms on the
    /// settled OneXFly against a 13.9 ms whole-frame budget — 6.3x
    /// SGSR 2's 1.868 on the same device, with FSR's own optimisations
    /// applied (`FFX_HALF`, its two-target history split, and the
    /// single-channel intermediates at four bytes instead of eight).
    /// 81 % of it is the accumulation pass, and nothing left in FSR's
    /// toolbox closes a factor of six.
    Fsr3,
}

impl UpscaleTechnique {
    /// Whether anything accumulates history, and therefore whether the
    /// camera jitters and the motion vectors are written.
    ///
    /// One predicate rather than `!= None` at each call site, because
    /// the question every pass actually asks is "is there a history",
    /// and a third temporal technique must not need those sites edited.
    pub fn is_temporal(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Whether the technique renders at a lower resolution than it
    /// presents. Distinct from [`Self::is_temporal`]: a resolve that
    /// only antialiases is temporal and not upscaling.
    pub fn upscales(self) -> bool {
        matches!(self, Self::Sgsr2 | Self::Fsr3)
    }

    /// The value as it is written in a `.rendersettings` file.
    ///
    /// 🔴 These numbers are serialised into user projects, so they are
    /// append-only: reordering the variants would silently change what
    /// an existing file means. Same rule as a renamed component.
    pub fn from_asset(value: u32) -> Self {
        match value {
            1 => Self::Taa,
            2 => Self::Sgsr2,
            3 => Self::Fsr3,
            _ => Self::None,
        }
    }
}

/// Which technique accumulates frames, as the project asked for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalSettings {
    pub technique: UpscaleTechnique,
    /// Render width as a percentage of the output's, 1..=100.
    ///
    /// 🔴 Already gated: [`RenderSettings::temporal`] forces this to 100
    /// unless the technique upscales, so nothing downstream has to ask
    /// twice. A resolve that cannot reconstruct handed a smaller frame
    /// produces a blurrier one and no speed the blit does not give back.
    pub render_scale: u32,
    /// How hard RCAS sharpens the finished image, 0..=100 (#481 step 5).
    ///
    /// ⚠️ Not temporal, and it travels here anyway: it exists because
    /// reconstruction is soft by construction, it is chosen in the same
    /// breath as the technique and the scale, and a settings bundle
    /// split three ways is three places to forget. What it is NOT is
    /// gated on the technique — a native frame may want a little of it,
    /// and a control that silently turns itself off when the upscaler
    /// changes is the footgun `render_scale` already had.
    pub sharpening: u32,
}

impl UpscaleTechnique {
    /// The render target's size for an output of `output`.
    ///
    /// Rounded down and floored at one: a window dragged to nothing must
    /// not ask for a zero-sized texture, which wgpu rejects outright.
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
    ///
    /// The stage builds its history pair either way — turning it on must
    /// not be the frame that stalls — but a temporal resolve rewrites
    /// every pixel of the image, and that is not something to adopt on
    /// behalf of a project that never mentioned it. The
    /// `.rendersettings` default is the same, and for a sharper reason:
    /// see `default_upscale` in `crate::settings`.
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
    /// Samples the texture filter takes along the long axis of a
    /// footprint, 1..=16 (1 = off).
    ///
    /// Here rather than beside the exposure because it is a **cost**
    /// setting: more fetches per sample on the surfaces that already
    /// cover the most pixels. What it buys is a floor that stays legible
    /// towards the horizon.
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
    ///
    /// See the module header for why the variable wins: it is the
    /// instrument, and an instrument whose reading depends on which
    /// project is open measures nothing.
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

impl TemporalSettings {
    /// `compute` is whether the compute shading path is on, and it is a
    /// gate on the scale for the same reason the technique is.
    ///
    /// 🔴 The fragment path tonemaps inline and shades straight into the
    /// image the window presents: it has no HDR target, so it has no
    /// intermediate at render resolution and nothing to resolve one
    /// from. Handed a smaller frame it mixes a depth buffer at render
    /// size with a colour target at window size in one pass, and **wgpu
    /// refuses the pass** — the frame is discarded whole, which is worse
    /// than the softness the gate on the technique exists to prevent.
    /// Reported from the editor at 1023x816 and 50 %.
    ///
    /// The same rule the shading rate already follows: turning compute
    /// off drops the rate back to full, because the fragment path has no
    /// thread to remove. This drops the scale back to 100, because it
    /// has nowhere to put a smaller frame.
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
            // Clamped once, here, for the same reason the scale is
            // gated once: every consumer downstream then gets a value
            // it can use without asking whether someone typed 500 into
            // a text file.
            sharpening: sharpening_override().unwrap_or(sharpening).min(100),
            // 🔴 The variable is still a BOOLEAN, and deliberately so.
            // It exists to force a technique on or off from a Steam
            // launch option while capturing on the handheld, where the
            // question is "what does this cost", not "which of three".
            // On, it selects the project's technique or falls back to
            // the resolve; off, it selects nothing.
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
///
/// The fifth variable of this shape and for the fifth time the same
/// reason: the editor is not where a temporal resolve can be judged.
/// What it costs is a full-screen pass on a handheld, and what it buys
/// is only visible while a camera is being moved by hand — neither of
/// which happens in a headless test or on a desktop GPU.
///
/// `None` when the variable says nothing, so the project's own setting
/// stands. Anything unrecognised is also `None`: a typo during a
/// measurement run must not silently decide which half of an A/B is
/// running.
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
///
/// The sixth variable of this shape, and the first one whose reason is
/// not cost but LOOK: what RCAS buys is only visible on a screen being
/// looked at, and the screen that matters is a handheld's at 1280x720
/// running a shipped build through Steam. There is no editor in that
/// process to move a slider in.
///
/// `None` when the variable says nothing or says something
/// unrecognised, so the project's own setting stands — a typo during a
/// capture must not silently decide which half of an A/B is running.
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
