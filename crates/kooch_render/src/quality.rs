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
    /// The same resolve, gathering the low-resolution samples into the
    /// output grid instead of reading one per pixel (#481).
    ///
    /// 🎯 TAAU is the CATEGORY and SGSR 2 is one implementation of it.
    /// This is ours, and it is the same shader as [`Self::Taa`] — at 1:1
    /// the two grids coincide, every weight collapses to one and it IS
    /// `Taa`. What differs from SGSR 2 is every decision inside: history
    /// clipped in YCoCg rather than clamped in RGB, disocclusion from a
    /// reversed-Z ratio rather than from AMD's tuned separation
    /// constant, and a range compressor that sees the exposure — which
    /// this engine cannot do without and mobile renderers never need.
    Taau,
    /// Snapdragon Game Super Resolution 2, transliterated (BSD-3).
    /// Resolves **and** upscales.
    ///
    /// ⚠️ Not offered in the inspector yet — the upscale pass is not
    /// built, so selecting it would be a menu entry that does nothing.
    /// The variant exists because the seam is what makes the second
    /// technique cheap, and building the seam with one implementation
    /// behind it is the whole point of doing it now.
    Sgsr2,
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
        matches!(self, Self::Taau | Self::Sgsr2)
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
            3 => Self::Taau,
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
        Self::new(UpscaleTechnique::None, 100)
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
}

impl Default for ShadingSettings {
    /// What the process environment says, so a measurement run that sets
    /// nothing else still behaves as it did before this module existed.
    fn default() -> Self {
        Self {
            compute: crate::meshlet::compute_shading_override().unwrap_or(false),
            rate: crate::meshlet::shading_rate_override().unwrap_or_default(),
        }
    }
}

impl ShadingSettings {
    /// The author's values with any `KOOCH_*` override applied on top.
    ///
    /// See the module header for why the variable wins: it is the
    /// instrument, and an instrument whose reading depends on which
    /// project is open measures nothing.
    pub fn from_asset(compute: bool, rate: ShadingRate) -> Self {
        Self {
            compute: crate::meshlet::compute_shading_override().unwrap_or(compute),
            rate: crate::meshlet::shading_rate_override().unwrap_or(rate),
        }
    }
}

impl TemporalSettings {
    pub fn new(technique: UpscaleTechnique, render_scale: u32) -> Self {
        Self {
            render_scale: if technique.upscales() {
                render_scale
            } else {
                100
            },
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 🔴 A technique that cannot reconstruct must not be handed a
    /// smaller frame.
    ///
    /// `None` and `TAA` both resolve at render resolution, so a scale
    /// under 100 there is a smaller image blown up by the blit: softer,
    /// and the speed goes back out through the upscale it cannot do.
    /// That is the classic way this setting earns a bad name, and it is
    /// refused here rather than documented as a footgun.
    #[test]
    fn only_an_upscaler_renders_smaller() {
        let out = (1920, 1080);
        assert_eq!(UpscaleTechnique::None.render_size(out, 50), out);
        assert_eq!(UpscaleTechnique::Taa.render_size(out, 50), out);
        assert_eq!(UpscaleTechnique::Sgsr2.render_size(out, 50), (960, 540));
    }

    /// And the gate is applied once, at the settings boundary, so
    /// nothing downstream has to remember to ask.
    #[test]
    fn the_settings_clamp_the_scale() {
        assert_eq!(
            TemporalSettings::new(UpscaleTechnique::Taa, 50).render_scale,
            100
        );
        assert_eq!(
            TemporalSettings::new(UpscaleTechnique::Sgsr2, 50).render_scale,
            50
        );
    }

    /// A window dragged to nothing must not ask wgpu for a zero-sized
    /// texture, which it rejects outright — the frame after a minimise
    /// would fail rather than render nothing.
    #[test]
    fn a_tiny_window_stays_renderable() {
        assert_eq!(UpscaleTechnique::Sgsr2.render_size((1, 1), 50), (1, 1));
        assert_eq!(UpscaleTechnique::Sgsr2.render_size((0, 0), 50), (1, 1));
    }

    /// 100 is the identity, and it is what every capture on record was
    /// taken at.
    #[test]
    fn native_scale_changes_nothing() {
        let out = (1280, 720);
        assert_eq!(UpscaleTechnique::Sgsr2.render_size(out, 100), out);
        assert_eq!(UpscaleTechnique::Sgsr2.render_size(out, 200), out);
    }
}
