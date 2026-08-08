//! Frame metrics in the window title.
//!
//! The only place a game can currently put text. `egui` belongs to the
//! editor and runtime UI is #96, so an on-screen overlay does not exist
//! to write into — while the title bar is one call away and is exactly
//! what every wgpu and winit example uses for this.
//!
//! Off unless `KOOCH_FRAME_METRICS` asks for `title` or `both`.

use std::time::{Duration, Instant};

use kooch_core::frame_metrics::FrameMetrics;
use kooch_core::resource::Resources;

use crate::WindowConfig;
use crate::handle::WindowHandle;

/// How often the title is rewritten.
///
/// Four times a second: fast enough to watch a number move, slow enough
/// that the text is readable and that the compositor is not asked to
/// redraw the decoration every frame.
const REFRESH: Duration = Duration::from_millis(250);

/// When the title was last rewritten. A resource rather than a `static`
/// so two windows would keep their own, and so a test can drive it.
#[derive(Debug, Default)]
pub struct TitleMetricsState {
    last: Option<Instant>,
}

impl TitleMetricsState {
    /// Whether a rewrite is due, marking it done if so.
    pub fn due(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.duration_since(last) < REFRESH => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

/// Appends the frame numbers to the configured window title.
///
/// The base title comes from [`WindowConfig`], never from the current
/// title: reading back what was written would append to the appended text
/// and grow the bar until it scrolled off the screen.
pub fn title_metrics_system(resources: &mut Resources) {
    let Some(metrics) = resources.get::<FrameMetrics>() else {
        return;
    };
    if !metrics.report.in_title() {
        return;
    }
    let summary = metrics.summary();

    let base = resources
        .get::<WindowConfig>()
        .map(|config| config.title.clone())
        .unwrap_or_default();

    let due = resources
        .get_mut::<TitleMetricsState>()
        .is_some_and(|state| state.due(Instant::now()));
    if !due {
        return;
    }

    if let Some(handle) = resources.get::<WindowHandle>() {
        handle.window().set_title(&format!("{base} — {summary}"));
    }
}

#[cfg(test)]
mod tests;
