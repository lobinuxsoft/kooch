//! Editor performance HUD — counters Resource + per-metric systems (#463).

pub(crate) mod breakdown;
pub(crate) mod persistence;
pub(crate) mod sys_metrics;
pub(crate) mod timing;

pub(crate) use breakdown::ms_since;
pub use breakdown::{
    FrameBreakdown, GatherStages, RenderStages, record_gizmo_batch_ms, record_render_stages,
};
pub(crate) use sys_metrics::{SysMetricsState, sys_metrics_system};
pub use timing::record_cpu_frame_ms;
pub(crate) use timing::{PerfTimingState, frame_timer_system};

/// Sampled-once-per-frame perf counters surfaced by the View toolbar HUD. Every field defaults to
/// zero so callers can read it before any metric system has run (boot frame, lazy refreshes, etc.)
/// and get a sane "n/a"-shaped value without unwrap juggling.
#[derive(Copy, Clone, Debug, Default)]
pub struct EditorPerfStats {
    /// Frame rate sampled from the most recent frame delta.
    pub fps_instant: f32,
    /// Frame rate averaged over the last 60 frames. Smoother for
    /// reading at a glance; instant catches stalls.
    pub fps_avg: f32,
    /// Wall-clock milliseconds between two frames, averaged over the same window as `fps_avg` — the
    /// WHOLE frame.
    pub frame_ms: f32,
    /// The longest frame in the same window `frame_ms` averages.
    pub worst_ms: f32,
    /// Wall-clock duration of the editor render system in
    /// milliseconds. Excludes GPU work AND everything outside the
    /// render system — see [`Self::frame_ms`].
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
    /// Whether the editor's own surface is presenting with vsync.
    pub vsync: bool,
    /// Sum of bytes the engine knows it has allocated through wgpu (vertex / index / uniform /
    /// storage buffers + textures we own, including the GlobalMeshPool, vis-buffer, deferred
    /// targets).
    pub vram_tracked_bytes: u64,
    /// Number of draw / dispatch submissions issued during the
    /// frame. Grows with every meshlet pool dispatch, sky pass,
    /// gizmo batch, and editor UI submit.
    pub draw_calls: u32,
    /// Cost of the remote snapshot pull, or `None` in local mode.
    pub remote: Option<RemoteSyncStats>,
    /// What the *project's* process reported its own frame to cost (#699). Distinct from everything
    /// else here, which describes the editor: when Play is pressed in remote mode, this is the
    /// process that is actually simulating, and until now nothing on screen came from it.
    pub host: Option<kooch_remote::protocol::HostMetrics>,
    /// Where `cpu_frame_ms` went, stage by stage (#691). Zeroed until
    /// the render system has completed a frame.
    pub breakdown: FrameBreakdown,
}

/// What one remote snapshot pull cost the editor's main thread (#645).
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
    /// Convenience for the HUD: ram in MB as `u32`, never panicking. Identical to reading the field
    /// directly — kept as a method so future format quirks (e.g. reporting "≥ 4 GB" when the count
    /// overflows u32 MB) can be handled without changing every call site.
    pub fn ram_rss_mb(&self) -> u32 {
        self.ram_rss_mb
    }

    /// Convenience for the HUD: VRAM tracked in MB.
    pub fn vram_tracked_mb(&self) -> u32 {
        (self.vram_tracked_bytes / (1024 * 1024)) as u32
    }
}

#[cfg(test)]
mod tests;

/// Whether anyone is looking at the metrics that cost something to take.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct HudVisibility {
    /// The **System** section inside it, which is the only reader of the
    /// sysinfo poll.
    #[serde(skip)]
    pub(crate) system_section: bool,
    /// The shadow-pages readout, as its OWN floating window: inlined in
    /// the Debug section it sat translucent over the 3D view and could
    /// not be read — the user's words were "no se entiende nada".
    pub(crate) shadow_pages_window: bool,
    /// Whether the Performance dock tab drew this frame. Set by the tab, cleared by
    /// `sys_metrics_system` after reading, so it is always at most one frame stale.
    #[serde(skip)]
    pub(crate) panel_visible: bool,
    /// Which sections have been pinned out into floating windows.
    pub(crate) pinned: PinnedSections,
    /// The Godot-style anchored cards on the game viewport, toggled from its View menu: frame
    /// timings top-right, render information bottom-right. Small, fixed, and out of the picture's
    /// way — the full readout lives in the Performance tab.
    pub(crate) frame_time_card: bool,
    pub(crate) info_card: bool,
}

/// One flag per pinnable section of the performance readout. A fixed
/// struct rather than a set: the sections are a closed list, and
/// `HudVisibility` stays `Copy` for the `PreRender` readers.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct PinnedSections {
    pub(crate) debug: bool,
    pub(crate) frame: bool,
    pub(crate) project: bool,
    pub(crate) system: bool,
    pub(crate) render: bool,
    pub(crate) meshlet: bool,
    pub(crate) cpu_frame: bool,
    pub(crate) remote: bool,
}

impl Default for HudVisibility {
    /// The overlay sidebar defaults HIDDEN now that the metrics live in the Performance dock tab:
    /// drawn over the game view they could not be read, which was the user's complaint.
    fn default() -> Self {
        Self {
            system_section: true,
            shadow_pages_window: false,
            panel_visible: false,
            pinned: PinnedSections::default(),
            // The one card a fresh layout shows: frame timings are what
            // everyone wants first, and everything else is a toggle
            // away in the View menu. The user's spec, verbatim.
            frame_time_card: true,
            info_card: false,
        }
    }
}

impl HudVisibility {
    /// Whether the OS is worth asking about CPU and memory this frame. Whether the OS is worth
    /// asking about CPU and memory this frame: the Performance tab is open with its System section
    /// expanded, or the System overlay card is on the game viewport.
    pub(crate) fn wants_system_metrics(self) -> bool {
        (self.panel_visible && self.system_section) || self.pinned.system
    }
}
