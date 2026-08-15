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

/// Whether frames are accumulated (#481).
///
/// One switch for two mechanisms, and they are not separable: the
/// sub-pixel jitter is what gives the resolve something to integrate,
/// and the resolve is what makes the jitter anything other than a
/// wobble. Half of the pair is always worse than neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalSettings {
    pub enabled: bool,
}

impl Default for TemporalSettings {
    /// Off.
    ///
    /// The stage builds its history pairs either way — turning it on
    /// must not be the frame that stalls — but a temporal resolve
    /// rewrites every pixel of the image, and that is not something to
    /// adopt on behalf of a project that never mentioned it. The
    /// `.rendersettings` default is the opposite, and deliberately: a
    /// project that ships an asset has an author who can see the result.
    fn default() -> Self {
        Self { enabled: false }
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
    /// How many of a froxel's lights each pixel evaluates (#826). Zero
    /// means all of them, which is exact and the most expensive thing
    /// the frame does.
    pub light_samples: u32,
}

impl Default for ShadingSettings {
    /// What the process environment says, so a measurement run that sets
    /// nothing else still behaves as it did before this module existed.
    fn default() -> Self {
        Self {
            compute: crate::meshlet::compute_shading_override().unwrap_or(false),
            rate: crate::meshlet::shading_rate_override().unwrap_or_default(),
            light_samples: kooch_lighting::light_samples_override().unwrap_or(0),
        }
    }
}

impl ShadingSettings {
    /// The author's values with any `KOOCH_*` override applied on top.
    ///
    /// See the module header for why the variable wins: it is the
    /// instrument, and an instrument whose reading depends on which
    /// project is open measures nothing.
    pub fn from_asset(compute: bool, rate: ShadingRate, light_samples: u32) -> Self {
        Self {
            compute: crate::meshlet::compute_shading_override().unwrap_or(compute),
            rate: crate::meshlet::shading_rate_override().unwrap_or(rate),
            light_samples: kooch_lighting::light_samples_override().unwrap_or(light_samples),
        }
    }
}

impl TemporalSettings {
    pub fn new(enabled: bool) -> Self {
        Self { enabled }
    }
}
