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
    /// Off, unless the environment says otherwise.
    ///
    /// The stage builds its history pair either way — turning it on must
    /// not be the frame that stalls — but a temporal resolve rewrites
    /// every pixel of the image, and that is not something to adopt on
    /// behalf of a project that never mentioned it. The
    /// `.rendersettings` default is the same, and for a sharper reason:
    /// see `default_temporal_aa` in `crate::settings`.
    fn default() -> Self {
        Self {
            enabled: temporal_aa_override().unwrap_or(false),
        }
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
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: temporal_aa_override().unwrap_or(enabled),
        }
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
