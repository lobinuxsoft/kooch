//! How many pixels share one shaded sample (#825).
//!
//! The raster stays at full resolution on every setting: geometry,
//! depth and the visibility buffer are untouched, and only the lighting
//! evaluation runs at a lower rate. That separation is what this issue
//! buys and it is only possible because #824 moved shading out of the
//! raster's fragment shader — while it lived there, its resolution *was*
//! the raster's.
//!
//! Distinct from #481 / #536, which upscale the **finished frame**.
//! These compose: this reduces the work per frame, FSR reduces the
//! pixels the frame is drawn at.

/// Pixels per shaded sample, per axis.
///
/// An enum and not a `u32`, because every value other than 1 and 2 is a
/// bug that would only show up as a wrong image on the device. Quarter
/// rate is deliberately absent until a capture asks for it: at 4x4 the
/// upsample's guide (one sample per 16 pixels) stops being able to
/// reconstruct a silhouette, and that is a different technique, not a
/// bigger constant.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum ShadingRate {
    /// One shaded sample per pixel. What every capture before #825 was
    /// taken against.
    #[default]
    Full,
    /// One shaded sample per 2x2 quad, upsampled back to full
    /// resolution using the visibility buffer as the edge guide.
    Half,
}

impl ShadingRate {
    /// Pixels per sample along one axis — 1 or 2. The dispatch size and
    /// the shader's pixel mapping both derive from this.
    pub fn factor(self) -> u32 {
        match self {
            Self::Full => 1,
            Self::Half => 2,
        }
    }

    /// The inverse of [`Self::factor`], for the settings asset, which
    /// stores the rate as the number an author reads (#830).
    ///
    /// Anything past 2 is [`Self::Half`] rather than an error: the
    /// reflected field is a plain `u32` and a project file can hold any
    /// of them, so the choice is between the nearest rate that exists
    /// and a panic on load.
    pub fn from_factor(factor: u32) -> Self {
        match factor {
            0 | 1 => Self::Full,
            _ => Self::Half,
        }
    }

    /// Dimensions of the shaded target for a screen of `size`.
    ///
    /// `div_ceil`, so an odd width still shades its last column: the
    /// quad that hangs off the right edge has one real pixel in it and
    /// the shader picks that one.
    pub fn target_size(self, size: (u32, u32)) -> (u32, u32) {
        let f = self.factor();
        (size.0.div_ceil(f), size.1.div_ceil(f))
    }

    /// Whether the shaded result needs the upsample pass to reach the
    /// screen.
    pub fn needs_upsample(self) -> bool {
        self != Self::Full
    }

    /// The label a GPU capture will show for this rate, so a capture
    /// answers "which rate produced it" without anybody trusting a log
    /// line — the same reason `shade: compute` / `shade: fragment` exist
    /// (#824).
    pub fn scope_label(self) -> &'static str {
        match self {
            Self::Full => "shade: compute",
            Self::Half => "shade: compute (half rate)",
        }
    }
}

/// `KOOCH_SHADING_RATE=half` (or `full`), read once.
///
/// 🔴 An environment variable **as well as** a live setter, and the
/// reasons differ. The setter exists because the player has to be able
/// to change this — graphics quality is a player-facing option, not a
/// build flag (#830). The environment variable exists because **the
/// editor is not where this can be measured**: the frame this issue
/// targets is a game on the OneXFly launched through Steam, and
/// `KOOCH_CLUSTERING`, `KOOCH_SPECULAR_FLOOR`, `KOOCH_COMPUTE_SHADING`
/// and `KOOCH_LIGHT_LIMIT` all learned that the same way.
///
/// Anything unrecognised is `None`, the same as unset: a typo during a
/// measurement run must not silently change which rate is being
/// measured, and — since the settings asset now supplies a value too
/// (#830) — must not silently override the author's either.
pub(crate) fn rate_from_environment() -> Option<ShadingRate> {
    static RATE: std::sync::OnceLock<Option<ShadingRate>> = std::sync::OnceLock::new();
    *RATE.get_or_init(|| {
        let rate = parse_rate(std::env::var("KOOCH_SHADING_RATE").ok().as_deref());
        if rate == Some(ShadingRate::Half) {
            tracing::info!(
                target: "kooch_render::vbuf64_stage",
                "KOOCH_SHADING_RATE=half: lighting runs at one sample per 2x2 quad, \
                 upsampled with the visibility buffer as the edge guide; the raster, \
                 depth and vbuf stay at full resolution",
            );
        }
        rate
    })
}

/// The parse, apart from the read, so a test can exercise it without
/// touching the process environment.
///
/// `None` means "the variable said nothing", which is what lets the
/// project's own setting stand.
fn parse_rate(raw: Option<&str>) -> Option<ShadingRate> {
    match raw {
        Some("half") | Some("2") => Some(ShadingRate::Half),
        Some("full") | Some("1") => Some(ShadingRate::Full),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
