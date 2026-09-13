//! How the game's window covers the display.
//!
//! The engine could set a window's title and its size and nothing else,
//! which meant a game built with it could not offer *"Windowed /
//! Borderless / Fullscreen"* — the entry that sits directly above vsync
//! in every graphics options menu ever shipped.
//!
//! # Why the type lives here and not beside the setting
//!
//! The value is authored in `.rendersettings`, which `kooch_render`
//! owns, and applied to a `winit::Window`, which `kooch_window` owns.
//! Neither crate depends on the other — both depend on this one — so
//! the shared vocabulary belongs here, the same way [`GpuContext`] does.
//!
//! [`GpuContext`]: crate::gpu::GpuContext
//!
//! # Exclusive fullscreen exists, and it does not work everywhere
//!
//! `winit::window::Fullscreen::Exclusive` takes a `VideoModeHandle` and
//! asks the display to **change mode** — the only way to alter the
//! output resolution from inside a process. Windows and X11 implement
//! it. **Wayland does not**, and winit's own source says so twice:
//!
//! ```text
//! Some(Fullscreen::Exclusive(_)) => {
//!     warn!("`Fullscreen::Exclusive` is ignored on Wayland");
//! },
//! ```
//!
//! It warns and changes nothing, which leaves the window exactly as it
//! was and reads as the setting being broken. So [`effective`] degrades
//! the request to [`WindowMode::Fullscreen`] here, and [`DisplayModes`]
//! reports whether the platform can honour it at all — a game's options
//! menu should not offer a resolution dropdown that does nothing.
//!
//! What controls the expensive resolution is `render_scale`, which is a
//! percentage of the output and already a setting.

/// Where the window sits between "a rectangle on a desktop" and "the
/// whole screen".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowMode {
    /// A normal decorated window at the size the project asked for.
    #[default]
    Windowed,
    /// The same size, without the title bar or the border.
    Borderless,
    /// Covering the monitor at the monitor's **current** mode.
    Fullscreen,
    /// Covering the monitor after asking it to **change mode** to [`Resolution`].
    Exclusive,
}

/// A resolution the game asks the display for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz. **Zero means "whatever the monitor
    /// offers at that size"**, which is what a resolution list without a
    /// refresh column has to mean, and what a window mode ignores.
    pub refresh_mhz: u32,
}

/// What the platform will actually do, published by `kooch_window` once the window exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DisplayModes {
    /// Every mode the current monitor reports, deduplicated and sorted
    /// largest first.
    pub modes: Vec<Resolution>,
    /// Whether [`WindowMode::Exclusive`] can be honoured here.
    pub exclusive: bool,
}

/// The mode that will actually be applied, given what the platform can do.
pub fn effective(wanted: WindowMode, exclusive_supported: bool) -> WindowMode {
    match wanted {
        WindowMode::Exclusive if !exclusive_supported => WindowMode::Fullscreen,
        mode => mode,
    }
}

/// Picks the display mode an [`WindowMode::Exclusive`] request should use, or `None` when the
/// monitor has nothing of that size.
pub fn best_mode(modes: &[Resolution], wanted: Resolution) -> Option<Resolution> {
    modes
        .iter()
        .filter(|mode| (mode.width, mode.height) == (wanted.width, wanted.height))
        .copied()
        .min_by_key(|mode| match wanted.refresh_mhz {
            0 => u32::MAX - mode.refresh_mhz,
            asked => mode.refresh_mhz.abs_diff(asked),
        })
}

impl WindowMode {
    /// The `.rendersettings` number, with `KOOCH_WINDOW_MODE` on top.
    pub fn from_asset(value: u32) -> Self {
        Self::resolve(
            match value {
                1 => Self::Borderless,
                2 => Self::Fullscreen,
                3 => Self::Exclusive,
                _ => Self::Windowed,
            },
            mode_override(),
        )
    }

    /// The precedence rule, apart from the read, so a test can exercise it without touching the
    /// process environment.
    fn resolve(asset: Self, over: Option<Self>) -> Self {
        over.unwrap_or(asset)
    }

    /// Whether the window covers the monitor.
    pub fn fullscreen(self) -> bool {
        matches!(self, Self::Fullscreen | Self::Exclusive)
    }

    /// Whether the window keeps its title bar and border.
    pub fn decorated(self) -> bool {
        matches!(self, Self::Windowed)
    }
}

/// `KOOCH_WINDOW_MODE`, read once. `None` means the variable said
/// nothing, which is what lets the project's own setting stand.
pub fn mode_override() -> Option<WindowMode> {
    static MODE: std::sync::OnceLock<Option<WindowMode>> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        let mode = mode_from(std::env::var("KOOCH_WINDOW_MODE").ok().as_deref());
        if let Some(mode) = mode {
            tracing::info!("KOOCH_WINDOW_MODE={mode:?}: overriding the project's window mode");
        }
        mode
    })
}

/// Reads the variable, so the rule is testable without an environment.
pub(crate) fn mode_from(raw: Option<&str>) -> Option<WindowMode> {
    match raw.map(str::trim) {
        Some("windowed") => Some(WindowMode::Windowed),
        Some("borderless") => Some(WindowMode::Borderless),
        Some("fullscreen") => Some(WindowMode::Fullscreen),
        Some("exclusive") => Some(WindowMode::Exclusive),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
