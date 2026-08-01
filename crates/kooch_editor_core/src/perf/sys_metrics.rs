//! System metrics — CPU% + RAM RSS via `sysinfo` (#463.3).
//!
//! Cross-platform (Linux / Windows / macOS). Sub-second sampling is
//! noise; we refresh at 500 ms intervals and the HUD reads the most
//! recent snapshot every frame. The `System` instance is reused
//! across calls (it caches OS handles) — recreating it per-frame
//! would be wasteful and would also miss the delta-based CPU%
//! computation entirely.

use std::time::{Duration, Instant};

use kooch_core::resource::Resources;
use sysinfo::{CpuRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

use super::EditorPerfStats;

/// How often we refresh CPU% / RAM measurements. Sub-second sampling
/// is just noise on a HUD; 500 ms catches sustained changes within
/// a frame or two while keeping the syscall budget negligible.
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

/// Persistent state owned by the sys-metrics system. Created once at
/// plugin build time so the cached `System` handle survives across
/// frames (CPU % needs the previous snapshot to compute a delta).
pub(crate) struct SysMetricsState {
    system: System,
    pid: Pid,
    last_refresh: Option<Instant>,
    /// How many refresh cycles have run since boot. sysinfo's
    /// `cpu_usage()` returns 0.0 on the very first refresh because
    /// it has no prior sample to delta against — we suppress writing
    /// stats until at least two refreshes have established a real
    /// baseline. RAM is exempt; one refresh is enough for it.
    samples_taken: u32,
}

impl Default for SysMetricsState {
    fn default() -> Self {
        // Build the System with the minimum refresh kinds we'll
        // actually query. Skipping disks / networks / components
        // keeps each refresh under a millisecond on Linux.
        //
        // Note the explicit `with_cpu(...)`: per-process cpu_usage
        // is computed against the GLOBAL cpu time baseline that
        // `System` tracks. Without enabling CPU on RefreshKind the
        // baseline never gets populated and `process.cpu_usage()`
        // returns 0.0 forever. The `with_processes(...)` part is
        // for memory + per-process times.
        let refresh_kind = RefreshKind::new()
            .with_cpu(CpuRefreshKind::new().with_cpu_usage())
            .with_processes(ProcessRefreshKind::new().with_cpu().with_memory());
        let system = System::new_with_specifics(refresh_kind);
        let pid = Pid::from_u32(std::process::id());
        Self {
            system,
            pid,
            last_refresh: None,
            samples_taken: 0,
        }
    }
}

/// PreRender system: refreshes the cached `System` snapshot at most
/// once per `REFRESH_INTERVAL`, then writes the current process's
/// CPU % + RSS into [`EditorPerfStats`].
///
/// Cheap on the no-refresh path: a single `Instant::elapsed` check
/// and we return without touching the OS. On the refresh path: one
/// `refresh_processes_specifics` call (μs-range on Linux) plus the
/// HashMap lookup of our own PID.
pub(crate) fn sys_metrics_system(resources: &mut Resources) {
    let mut state = resources.remove::<SysMetricsState>().unwrap_or_default();

    let should_refresh = match state.last_refresh {
        None => true,
        Some(prev) => prev.elapsed() >= REFRESH_INTERVAL,
    };

    if should_refresh {
        // Refresh global CPU usage FIRST so the per-process delta
        // computed below has a fresh global baseline to compare
        // against. Without this, `process.cpu_usage()` returns
        // 0.0 even after multiple refreshes — empirically observed
        // on Linux during #463 manual validation.
        state
            .system
            .refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage());
        state.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[state.pid]),
            true,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        state.last_refresh = Some(Instant::now());
        state.samples_taken = state.samples_taken.saturating_add(1);

        if let Some(proc) = state.system.process(state.pid) {
            // RAM is a snapshot — usable from the very first sample.
            let ram_bytes = proc.memory();
            let ram_rss_mb = (ram_bytes / (1024 * 1024)) as u32;
            // CPU usage requires a baseline + delta. The first sysinfo
            // refresh records the baseline and `cpu_usage()` returns
            // 0.0; the second refresh produces a real delta. Skipping
            // the first sample means the HUD shows the previous
            // (zero) value for the first interval and a real number
            // afterward, instead of a misleading "0% forever" if the
            // baseline write happens to be the only sample seen.
            let cpu_percent = if state.samples_taken >= 2 {
                Some(proc.cpu_usage())
            } else {
                None
            };
            if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
                stats.ram_rss_mb = ram_rss_mb;
                if let Some(c) = cpu_percent {
                    stats.cpu_percent = c;
                }
            }
        }
    }

    resources.insert(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_call_populates_ram_eventually() {
        // Sysinfo's first cpu_usage() reading is typically 0.0 (no
        // delta yet), but RAM is available immediately. Verify that
        // running the system at least populates the RAM field.
        let mut resources = Resources::default();
        resources.insert(EditorPerfStats::default());
        sys_metrics_system(&mut resources);
        let stats = resources.get::<EditorPerfStats>().unwrap();
        assert!(
            stats.ram_rss_mb > 0,
            "expected the editor process to report some RSS, got {}",
            stats.ram_rss_mb
        );
    }

    #[test]
    fn second_call_inside_refresh_interval_is_a_noop() {
        // Two calls back-to-back: only the first should hit the OS.
        // We can't observe the syscall count directly, but we CAN
        // observe that `last_refresh` doesn't move on the second
        // call.
        let mut resources = Resources::default();
        resources.insert(EditorPerfStats::default());
        sys_metrics_system(&mut resources);
        let first = resources
            .get::<SysMetricsState>()
            .unwrap()
            .last_refresh
            .unwrap();
        sys_metrics_system(&mut resources);
        let second = resources
            .get::<SysMetricsState>()
            .unwrap()
            .last_refresh
            .unwrap();
        assert_eq!(first, second, "back-to-back call must not refresh again");
    }

    #[test]
    fn first_sample_does_not_overwrite_cpu_percent() {
        // sysinfo's first refresh always reports cpu_usage = 0.0
        // because there's no prior sample to delta against. If the
        // HUD reads this value it will display a stuck "0.0 %"
        // until enough idle time passes for a second refresh — the
        // user reported exactly this. Verify the first refresh does
        // not overwrite an existing non-zero value.
        let mut resources = Resources::default();
        let mut seeded_stats = EditorPerfStats::default();
        seeded_stats.cpu_percent = 42.0; // simulate prior reading
        resources.insert(seeded_stats);
        sys_metrics_system(&mut resources);
        let stats = resources.get::<EditorPerfStats>().unwrap();
        assert_eq!(
            stats.cpu_percent, 42.0,
            "first sample must NOT overwrite a non-zero cpu_percent (sysinfo returns 0 \
             on the first refresh; only the second has a real delta)"
        );
        // RAM, in contrast, is a snapshot — should populate even
        // from the first sample.
        assert!(
            stats.ram_rss_mb > 0,
            "RAM must populate from the very first sample"
        );
    }
}

#[cfg(test)]
mod measure {
    use super::*;

    /// What one refresh costs, on the frame it lands on (#703).
    ///
    /// It runs twice a second whether or not the System section is open,
    /// and unlike the cull counters it is gated on nothing. Gating it on
    /// visibility is only worth doing if this number says so.
    ///
    /// ```text
    /// cargo test -p kooch_editor_core --lib sys_metrics -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "measures; run with --ignored --nocapture"]
    fn what_a_refresh_costs() {
        let mut state = SysMetricsState::default();
        // First refresh builds the baseline and is not what a steady
        // frame pays.
        state
            .system
            .refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage());

        const SAMPLES: usize = 20;
        let start = std::time::Instant::now();
        for _ in 0..SAMPLES {
            state
                .system
                .refresh_cpu_specifics(CpuRefreshKind::new().with_cpu_usage());
            state.system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[state.pid]),
                true,
                ProcessRefreshKind::new().with_cpu().with_memory(),
            );
        }
        let per_refresh = start.elapsed().as_secs_f64() * 1000.0 / SAMPLES as f64;
        println!("\n  sysinfo refresh: {per_refresh:.3} ms, twice a second\n");
    }
}
