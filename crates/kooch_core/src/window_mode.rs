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
//! # Why there is no exclusive fullscreen
//!
//! `winit::window::Fullscreen` has two variants and only one of them is
//! here. `Exclusive` takes a `VideoModeHandle` and asks the display to
//! *change mode*, which is the only way to alter the output resolution
//! from inside a process — and it is also the one that does not exist on
//! this engine's primary platform. **Wayland has no client-side mode
//! setting**: the compositor owns modes, and under gamescope it owns
//! the scaling too. Offering a mode that silently degrades to borderless
//! on Linux is worse than not offering it, and enumerating video modes
//! is a feature of its own rather than a variant of this one.
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
    ///
    /// 🔴 Not the same as [`Self::Fullscreen`], which is the mistake the
    /// name invites: this one still occupies `width x height`. It is
    /// what a game uses to draw its own chrome.
    Borderless,
    /// Covering the monitor at the monitor's **current** mode.
    ///
    /// No mode change, so no black screen on alt-tab and no resolution
    /// left wrong when the process dies — which is why every recent game
    /// defaults to this rather than to exclusive.
    Fullscreen,
}

impl WindowMode {
    /// The `.rendersettings` number, with `KOOCH_WINDOW_MODE` on top.
    ///
    /// 🔴 The numbers are serialised into user projects and are
    /// therefore append-only, the same rule `upscale` carries.
    /// Anything unrecognised is [`Self::Windowed`]: a file from a newer
    /// engine must open in the mode that is always available rather than
    /// take the display.
    pub fn from_asset(value: u32) -> Self {
        Self::resolve(
            match value {
                1 => Self::Borderless,
                2 => Self::Fullscreen,
                _ => Self::Windowed,
            },
            mode_override(),
        )
    }

    /// The precedence rule, apart from the read, so a test can exercise
    /// it without touching the process environment.
    ///
    /// The variable wins, for the reason `kooch_render::quality` gives
    /// at length: a measurement run must get what it asked for whichever
    /// project happens to be open.
    fn resolve(asset: Self, over: Option<Self>) -> Self {
        over.unwrap_or(asset)
    }

    /// Whether the window covers the monitor.
    pub fn fullscreen(self) -> bool {
        matches!(self, Self::Fullscreen)
    }

    /// Whether the window keeps its title bar and border.
    ///
    /// A fullscreen window is undecorated by definition, and saying so
    /// here rather than at the call site keeps the two answers from
    /// disagreeing.
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
///
/// Anything unrecognised is `None` rather than a guess: a typo must not
/// take the display, and must not silently override the author either.
pub(crate) fn mode_from(raw: Option<&str>) -> Option<WindowMode> {
    match raw.map(str::trim) {
        Some("windowed") => Some(WindowMode::Windowed),
        Some("borderless") => Some(WindowMode::Borderless),
        Some("fullscreen") => Some(WindowMode::Fullscreen),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
