//! Editor performance HUD — counters Resource + per-metric systems (#463).
//!
//! Cross-platform stack: every metric here lands via portable
//! mechanisms (manual frame timer, `sysinfo` for process CPU/RAM,
//! wgpu timestamp queries for GPU frame time, engine-side counters
//! for VRAM-tracked and draw calls). No vendor-specific code, no
//! system dependencies.
//!
//! The Resource is pure data — population happens in dedicated
//! systems wired by [`crate::EditorPlugin`]. Reads are zero-cost: a
//! single `resources.get::<EditorPerfStats>()` from the View toolbar.

pub(crate) mod sys_metrics;
pub(crate) mod timing;

pub(crate) use sys_metrics::{SysMetricsState, sys_metrics_system};
pub use timing::record_cpu_frame_ms;
pub(crate) use timing::{PerfTimingState, frame_timer_system};

/// Sampled-once-per-frame perf counters surfaced by the View toolbar
/// HUD. Every field defaults to zero so callers can read it before
/// any metric system has run (boot frame, lazy refreshes, etc.) and
/// get a sane "n/a"-shaped value without unwrap juggling.
///
/// `f32` for everything time-/percent-shaped (display ergonomics —
/// the HUD never needs sub-microsecond precision and bytemuck-
/// compatible POD layout is irrelevant since this never crosses the
/// CPU/GPU boundary). Counts use `u32` / `u64` per their natural
/// range.
///
/// `gpu_frame_ms == None` means the active adapter does not report
/// `Features::TIMESTAMP_QUERY` — the HUD renders "GPU n/a" without
/// hiding the rest of the row.
#[derive(Copy, Clone, Debug, Default)]
pub struct EditorPerfStats {
    /// Frame rate sampled from the most recent frame delta.
    pub fps_instant: f32,
    /// Frame rate averaged over the last 60 frames. Smoother for
    /// reading at a glance; instant catches stalls.
    pub fps_avg: f32,
    /// Wall-clock duration of the editor render system in
    /// milliseconds. Excludes GPU work.
    pub cpu_frame_ms: f32,
    /// Sampled CPU usage of the editor process (0.0..=100.0 across
    /// all cores summed, matches `top`'s convention). Refreshed at
    /// most twice per second — sub-second sampling is noise.
    pub cpu_percent: f32,
    /// Resident set size of the editor process in megabytes. RSS
    /// (not virtual) is what the OS will actually evict under
    /// memory pressure.
    pub ram_rss_mb: u32,
    /// Wall-clock GPU pass duration in milliseconds, measured via
    /// `wgpu::Features::TIMESTAMP_QUERY`. `None` if the adapter does
    /// not expose timestamp queries.
    pub gpu_frame_ms: Option<f32>,
    /// Sum of bytes the engine knows it has allocated through wgpu
    /// (vertex / index / uniform / storage buffers + textures we
    /// own, including the GlobalMeshPool, vis-buffer, deferred
    /// targets). Does NOT include driver overhead, swap chain, or
    /// implicit allocations — that requires per-backend queries
    /// the wgpu API does not expose portably.
    pub vram_tracked_bytes: u64,
    /// Number of draw / dispatch submissions issued during the
    /// frame. Grows with every meshlet pool dispatch, sky pass,
    /// gizmo batch, and editor UI submit.
    pub draw_calls: u32,
    /// Cost of the remote snapshot pull, or `None` in local mode.
    pub remote: Option<RemoteSyncStats>,
}

/// What one remote snapshot pull cost the editor's main thread (#645).
///
/// Every field describes the **last pull**, not the last frame. The
/// pull runs on a cadence — every frame while playing, one frame in
/// thirty while idle — and holding the previous sample keeps the HUD
/// readable instead of flickering to zero on the twenty-nine frames
/// that skip it.
///
/// `refresh_ms` is the whole main-thread stall and is what the frame
/// budget actually pays. It is not the sum of `transport_ms` and
/// `decode_ms`: those come from the client's last call, so on a frame
/// where the refresh was skipped they describe an older call. Read the
/// split as *which half dominates*, not as an exact decomposition.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct RemoteSyncStats {
    /// Wall-clock of `session.refresh()` — socket, the wait for the
    /// project's next `Stage::First`, and the parse.
    pub refresh_ms: f32,
    /// The socket half: connect, write, and block until the server's
    /// main thread answers. Dominant here means the editor is waiting
    /// on the project's frame boundary, not on bandwidth.
    pub transport_ms: f32,
    /// The parse half. Dominant here means the payload is too big and
    /// the answer is to send less, not to send it off-thread.
    pub decode_ms: f32,
    /// Wall-clock of `mirror.apply()` — rebuilding the snapshot into
    /// the editor's own ECS. Runs on the same frames as the pull and
    /// was equally unmeasured.
    pub mirror_ms: f32,
    /// Entities in the snapshot: the denominator without which the
    /// milliseconds above mean nothing.
    pub entities: u32,
    /// Bytes of the last response body.
    pub snapshot_bytes: u32,
}

impl EditorPerfStats {
    /// Convenience for the HUD: ram in MB as `u32`, never panicking.
    /// Identical to reading the field directly — kept as a method so
    /// future format quirks (e.g. reporting "≥ 4 GB" when the count
    /// overflows u32 MB) can be handled without changing every call
    /// site.
    pub fn ram_rss_mb(&self) -> u32 {
        self.ram_rss_mb
    }

    /// Convenience for the HUD: VRAM tracked in MB.
    pub fn vram_tracked_mb(&self) -> u32 {
        (self.vram_tracked_bytes / (1024 * 1024)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_zeroed_with_none_gpu_ms() {
        let s = EditorPerfStats::default();
        assert_eq!(s.fps_instant, 0.0);
        assert_eq!(s.fps_avg, 0.0);
        assert_eq!(s.cpu_frame_ms, 0.0);
        assert_eq!(s.cpu_percent, 0.0);
        assert_eq!(s.ram_rss_mb, 0);
        assert_eq!(s.gpu_frame_ms, None);
        assert_eq!(s.vram_tracked_bytes, 0);
        assert_eq!(s.draw_calls, 0);
        assert_eq!(s.remote, None, "local mode reports no remote cost");
    }

    #[test]
    fn vram_tracked_mb_truncates_correctly() {
        let mut s = EditorPerfStats::default();
        s.vram_tracked_bytes = 5 * 1024 * 1024 + 512; // 5 MB + a bit
        assert_eq!(s.vram_tracked_mb(), 5);
    }
}
