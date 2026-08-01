//! What a frame cost, measured by the engine rather than by the editor.
//!
//! Every timer in this project used to live in `kooch_editor_core`, which
//! meant a game running on its own — `cargo run -- --game`, or an exported
//! build — had no idea how fast it was going. The person who most needs
//! that number was the only one who could not get it (#698).
//!
//! # Two numbers, and why neither replaces the other
//!
//! [`FrameMetrics::frame_ms`] is wall-clock time between frame starts:
//! what a player feels. With vsync on it sits at the refresh interval and
//! stays there whether the frame had 2 ms of work or 15 — which is why
//! reading only this one hides the moment before a drop.
//!
//! [`FrameMetrics::cpu_frame_ms`] is the work inside the frame, with the
//! wait for the next one excluded. It moves when the game gets heavier,
//! vsync or not, and it is the number to watch while optimising.
//!
//! # Reporting is opt-in; collecting is not
//!
//! Collecting costs two subtractions and an `Instant::elapsed`, so it
//! always runs. Reporting is off unless `KOOCH_FRAME_METRICS` says
//! otherwise — an environment variable rather than a plugin the project
//! has to add, so that reading the number never requires editing a game
//! to ask the question.
//!
//! ```text
//! KOOCH_FRAME_METRICS=log    cargo run -- --game   # a line per second
//! KOOCH_FRAME_METRICS=title  cargo run -- --game   # in the window title
//! KOOCH_FRAME_METRICS=both   cargo run -- --game
//! ```
//!
//! There is no on-screen overlay: a game cannot draw text. `egui` is the
//! editor's, and runtime UI is #96. Anything that renders these numbers
//! into the viewport is blocked on that, and says so rather than being
//! quietly attempted.

use std::time::{Duration, Instant};

use crate::resource::Resources;
use crate::time::Time;

/// How many frames the rolling average covers.
///
/// A second-ish at 60 Hz. Short enough to follow a scene getting heavier,
/// long enough that one late frame does not swing the reading.
const AVERAGE_WINDOW: usize = 60;

/// How the numbers reach a person, from `KOOCH_FRAME_METRICS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MetricsReport {
    /// Collected and readable in the resource, shown nowhere.
    #[default]
    Silent,
    /// A `tracing` line at a fixed interval. Goes to stdout for a game run
    /// from a terminal, and to the editor's Console through the pipe it
    /// already reads.
    Log,
    /// Appended to the window title. What every wgpu example does, and the
    /// only place a windowed game can currently put text.
    Title,
    Both,
}

impl MetricsReport {
    /// Reads `KOOCH_FRAME_METRICS`. An unrecognised value is `Silent` with
    /// a warning: silently ignoring a misspelling would look exactly like
    /// the feature not existing.
    pub fn from_env() -> Self {
        let Some(raw) = std::env::var_os("KOOCH_FRAME_METRICS") else {
            return Self::Silent;
        };
        match raw.to_string_lossy().to_ascii_lowercase().as_str() {
            "" | "0" | "off" | "silent" => Self::Silent,
            "1" | "log" => Self::Log,
            "title" => Self::Title,
            "both" => Self::Both,
            other => {
                tracing::warn!(
                    "KOOCH_FRAME_METRICS={other} is not one of log, title, both, off — \
                     frame metrics stay silent"
                );
                Self::Silent
            }
        }
    }

    pub fn logs(self) -> bool {
        matches!(self, Self::Log | Self::Both)
    }

    pub fn in_title(self) -> bool {
        matches!(self, Self::Title | Self::Both)
    }
}

/// What the last frame cost.
///
/// Written once per frame in [`Stage::Last`](crate::stage::Stage::Last),
/// which is the one place both runners agree on — the windowed loop and
/// the headless one each have their own tick, and a timer placed in either
/// would measure only half the ways this engine runs.
#[derive(Debug, Clone)]
pub struct FrameMetrics {
    /// Wall-clock milliseconds between frame starts, waiting included.
    pub frame_ms: f32,
    /// Milliseconds of work in the frame, waiting excluded.
    pub cpu_frame_ms: f32,
    /// GPU milliseconds, when the renderer reports them.
    ///
    /// `None` on a host with no renderer, and on an adapter without
    /// timestamp queries. Written by `kooch_render` rather than measured
    /// here: it already runs the queries, and a second set would be a
    /// second answer to the same question.
    pub gpu_frame_ms: Option<f32>,
    /// Frames per second from this frame alone. Jumpy on purpose — it is
    /// what catches a single stall.
    pub fps_instant: f32,
    /// Frames per second over the last [`AVERAGE_WINDOW`] frames.
    pub fps_average: f32,
    /// How the numbers are being reported, resolved once at startup.
    pub report: MetricsReport,

    /// Rolling window of frame durations, in seconds.
    recent: Vec<f32>,
    /// Where the next sample goes — a ring, so the window costs nothing
    /// to advance.
    next: usize,
    /// When the last log line went out.
    last_report: Option<Instant>,
}

impl Default for FrameMetrics {
    fn default() -> Self {
        Self {
            frame_ms: 0.0,
            cpu_frame_ms: 0.0,
            gpu_frame_ms: None,
            fps_instant: 0.0,
            fps_average: 0.0,
            report: MetricsReport::Silent,
            recent: Vec::with_capacity(AVERAGE_WINDOW),
            next: 0,
            last_report: None,
        }
    }
}

impl FrameMetrics {
    pub fn new(report: MetricsReport) -> Self {
        Self {
            report,
            ..Self::default()
        }
    }

    /// Records one frame. `work` is what happened inside it; `wall` is the
    /// gap to the previous frame's start.
    pub fn record(&mut self, wall: Duration, work: Duration) {
        self.frame_ms = wall.as_secs_f32() * 1000.0;
        self.cpu_frame_ms = work.as_secs_f32() * 1000.0;

        let seconds = wall.as_secs_f32();
        // The first frame's delta is whatever `Time` was constructed with,
        // and a zero would divide into infinity. Reported as zero FPS,
        // which reads as "no measurement yet" rather than as a number.
        self.fps_instant = if seconds > 0.0 { 1.0 / seconds } else { 0.0 };

        if self.recent.len() < AVERAGE_WINDOW {
            self.recent.push(seconds);
        } else {
            self.recent[self.next] = seconds;
            self.next = (self.next + 1) % AVERAGE_WINDOW;
        }
        let total: f32 = self.recent.iter().sum();
        self.fps_average = if total > 0.0 {
            self.recent.len() as f32 / total
        } else {
            0.0
        };
    }

    /// Whether a log line is due, and marks it sent if so.
    ///
    /// Takes `now` rather than reading the clock so the decision is
    /// testable without sleeping.
    pub fn should_log(&mut self, now: Instant, every: Duration) -> bool {
        match self.last_report {
            Some(last) if now.duration_since(last) < every => false,
            _ => {
                self.last_report = Some(now);
                true
            }
        }
    }

    /// One line's worth of the numbers, for a log or a title bar.
    pub fn summary(&self) -> String {
        let gpu = match self.gpu_frame_ms {
            Some(ms) => format!(", gpu {ms:.2} ms"),
            None => String::new(),
        };
        format!(
            "{:.0} fps ({:.0} avg) — frame {:.2} ms, cpu {:.2} ms{gpu}",
            self.fps_instant, self.fps_average, self.frame_ms, self.cpu_frame_ms,
        )
    }
}

/// How often the log form prints.
const LOG_EVERY: Duration = Duration::from_secs(1);

/// Measures the frame and, if asked, says what it cost.
///
/// Runs in `Stage::Last`, after rendering and before the loop waits.
pub fn frame_metrics_system(resources: &mut Resources) {
    let Some((wall, work)) = resources
        .get::<Time>()
        .map(|time| (time.delta(), time.frame_start().elapsed()))
    else {
        return;
    };

    let Some(metrics) = resources.get_mut::<FrameMetrics>() else {
        return;
    };
    metrics.record(wall, work);

    if metrics.report.logs() && metrics.should_log(Instant::now(), LOG_EVERY) {
        tracing::info!(target: "kooch::frame", "{}", metrics.summary());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_of_sixteen_milliseconds_is_sixty_fps() {
        let mut metrics = FrameMetrics::default();
        metrics.record(Duration::from_micros(16_667), Duration::from_millis(4));
        assert!((metrics.fps_instant - 60.0).abs() < 0.1);
        assert!((metrics.frame_ms - 16.667).abs() < 0.01);
        assert!((metrics.cpu_frame_ms - 4.0).abs() < 0.01);
    }

    /// The whole reason `cpu_frame_ms` exists: under vsync the wall time
    /// is pinned to the refresh interval, so a game doing four times the
    /// work reports the same FPS right up until it misses.
    #[test]
    fn work_moves_when_the_wall_clock_cannot() {
        let mut light = FrameMetrics::default();
        light.record(Duration::from_micros(16_667), Duration::from_millis(2));
        let mut heavy = FrameMetrics::default();
        heavy.record(Duration::from_micros(16_667), Duration::from_millis(8));

        assert_eq!(light.fps_instant, heavy.fps_instant);
        assert!(heavy.cpu_frame_ms > light.cpu_frame_ms * 3.0);
    }

    /// A stall shows in the instant reading and is diluted in the average.
    #[test]
    fn one_late_frame_does_not_swing_the_average() {
        let mut metrics = FrameMetrics::default();
        for _ in 0..AVERAGE_WINDOW {
            metrics.record(Duration::from_micros(16_667), Duration::from_millis(4));
        }
        metrics.record(Duration::from_millis(100), Duration::from_millis(90));

        assert!(metrics.fps_instant < 11.0, "the stall is visible");
        assert!(
            metrics.fps_average > 50.0,
            "the average is not dragged down"
        );
    }

    /// The window is a ring: a slow patch has to age out, not accumulate
    /// forever in a Vec that grows for the life of the process.
    #[test]
    fn the_window_stays_the_size_it_says() {
        let mut metrics = FrameMetrics::default();
        for _ in 0..AVERAGE_WINDOW * 3 {
            metrics.record(Duration::from_micros(16_667), Duration::from_millis(4));
        }
        assert_eq!(metrics.recent.len(), AVERAGE_WINDOW);
        assert!((metrics.fps_average - 60.0).abs() < 0.5);
    }

    #[test]
    fn a_zero_length_frame_reports_no_measurement_rather_than_infinity() {
        let mut metrics = FrameMetrics::default();
        metrics.record(Duration::ZERO, Duration::ZERO);
        assert_eq!(metrics.fps_instant, 0.0);
        assert!(metrics.fps_average.is_finite());
    }

    #[test]
    fn logging_waits_out_its_interval() {
        let mut metrics = FrameMetrics::default();
        let start = Instant::now();
        assert!(metrics.should_log(start, LOG_EVERY), "the first is due");
        assert!(!metrics.should_log(start + Duration::from_millis(500), LOG_EVERY));
        assert!(metrics.should_log(start + Duration::from_millis(1500), LOG_EVERY));
    }

    #[test]
    fn the_gpu_number_is_absent_rather_than_zero_when_nothing_reports_it() {
        let metrics = FrameMetrics::default();
        assert_eq!(metrics.gpu_frame_ms, None);
        assert!(!metrics.summary().contains("gpu"));
    }
}
