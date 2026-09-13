//! How many pixels share one shaded sample (#825).

/// Pixels per shaded sample, per axis.
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

    /// The inverse of [`Self::factor`], for the settings asset, which stores the rate as the number
    /// an author reads (#830).
    pub fn from_factor(factor: u32) -> Self {
        match factor {
            0 | 1 => Self::Full,
            _ => Self::Half,
        }
    }

    /// Dimensions of the shaded target for a screen of `size`.
    pub fn target_size(self, size: (u32, u32)) -> (u32, u32) {
        let f = self.factor();
        (size.0.div_ceil(f), size.1.div_ceil(f))
    }

    /// Whether the shaded result needs the upsample pass to reach the
    /// screen.
    pub fn needs_upsample(self) -> bool {
        self != Self::Full
    }

    /// The label a GPU capture will show for this rate, so a capture answers "which rate produced
    /// it" without anybody trusting a log line — the same reason `shade: compute` / `shade:
    /// fragment` exist (#824).
    pub fn scope_label(self) -> &'static str {
        match self {
            Self::Full => "shade: compute",
            Self::Half => "shade: compute (half rate)",
        }
    }
}

/// `KOOCH_SHADING_RATE=half` (or `full`), read once.
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

/// The parse, apart from the read, so a test can exercise it without touching the process
/// environment.
fn parse_rate(raw: Option<&str>) -> Option<ShadingRate> {
    match raw {
        Some("half") | Some("2") => Some(ShadingRate::Half),
        Some("full") | Some("1") => Some(ShadingRate::Full),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
