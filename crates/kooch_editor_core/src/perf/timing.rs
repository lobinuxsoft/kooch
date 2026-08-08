//! Frame timer + FPS computation (#463.2).
//!
//! Splits two distinct measurements that the HUD displays as one row:
//!
//! - **FPS** is derived from wall-clock delta between successive
//!   editor render system invocations. Captures the FULL frame
//!   (CPU work + GPU submit + present + idle waiting on vsync), so
//!   it tracks what the user actually sees on screen.
//! - **CPU frame ms** is the duration of the editor render system
//!   itself (start to end of the function). Excludes GPU work and
//!   vsync stalls. Useful for spotting CPU-side regressions even
//!   when FPS sits at the refresh rate.
//!
//! Both populate [`crate::perf::EditorPerfStats`]. The FPS path
//! lives in a dedicated `frame_timer_system` running in
//! [`Stage::PreRender`] so it sees the wall-clock spacing between
//! frames; the CPU path is written by [`crate::perf::record_cpu_frame_ms`]
//! invoked from inside the render system.

use std::collections::VecDeque;
use std::time::Instant;

use kooch_core::resource::Resources;

use super::EditorPerfStats;

/// Number of recent frame deltas the rolling average uses. 60 ≈ 1 s
/// at 60 Hz, smooth enough for a HUD glance, short enough that a
/// stutter shows up within a second.
const FPS_AVG_WINDOW: usize = 60;

/// Internal state for the frame timer. Not exposed publicly — the
/// HUD reads via [`EditorPerfStats`].
#[derive(Default)]
pub(crate) struct PerfTimingState {
    /// Timestamp captured the previous time `frame_timer_system` ran.
    /// `None` on the very first frame so we don't report a giant
    /// initial delta from process boot.
    last_frame_start: Option<Instant>,
    /// Recent inter-frame deltas in milliseconds. Bounded ring of
    /// at most `FPS_AVG_WINDOW` entries; oldest entry drops on
    /// overflow.
    frame_ms_history: VecDeque<f32>,
}

/// PreRender system: measures the wall-clock delta since the last
/// invocation and writes `fps_instant` + `fps_avg` into the perf
/// stats Resource. Skips the first frame (no previous timestamp to
/// diff against) so the average stays clean.
pub(crate) fn frame_timer_system(resources: &mut Resources) {
    let now = Instant::now();
    let mut state = resources.remove::<PerfTimingState>().unwrap_or_default();

    if let Some(prev) = state.last_frame_start {
        let frame_ms = now.duration_since(prev).as_secs_f32() * 1000.0;
        // Guard against pathological deltas (e.g. the editor was
        // suspended / debugger paused). Anything > 1 second of
        // silence resets the moving average so we don't drag a
        // multi-second outlier through the window.
        if frame_ms < 1000.0 {
            if state.frame_ms_history.len() == FPS_AVG_WINDOW {
                state.frame_ms_history.pop_front();
            }
            state.frame_ms_history.push_back(frame_ms);

            let fps_instant = if frame_ms > 0.0 {
                1000.0 / frame_ms
            } else {
                0.0
            };
            let avg_ms: f32 = state.frame_ms_history.iter().copied().sum::<f32>()
                / state.frame_ms_history.len() as f32;
            let fps_avg = if avg_ms > 0.0 { 1000.0 / avg_ms } else { 0.0 };

            if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
                stats.fps_instant = fps_instant;
                stats.fps_avg = fps_avg;
            }
        } else {
            state.frame_ms_history.clear();
        }
    }

    state.last_frame_start = Some(now);
    resources.insert(state);
}

/// Records the duration of the editor render system into
/// `EditorPerfStats.cpu_frame_ms`. Call sites pass the `Instant`
/// captured at the very top of the render function and invoke this
/// just before the function returns. Keeping the helper external to
/// `frame_timer_system` lets the CPU measurement stay tight (no
/// system-boundary overhead on the timing path).
pub fn record_cpu_frame_ms(resources: &mut Resources, frame_start: Instant) {
    let elapsed_ms = frame_start.elapsed().as_secs_f32() * 1000.0;
    if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
        stats.cpu_frame_ms = elapsed_ms;
    }
}

#[cfg(test)]
mod tests;
