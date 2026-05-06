//! System metrics — CPU% + RAM RSS via `sysinfo` (#463.3).
//!
//! Cross-platform (Linux / Windows / macOS). Sub-second sampling is
//! noise; we refresh at 500 ms intervals and the HUD reads the most
//! recent snapshot every frame. The `System` instance is reused
//! across calls (it caches OS handles) — recreating it per-frame
//! would be wasteful and would also miss the delta-based CPU%
//! computation entirely.

use std::time::{Duration, Instant};

use ome_core::resource::Resources;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};

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
}

impl Default for SysMetricsState {
    fn default() -> Self {
        // Build the System with the minimum refresh kinds we'll
        // actually query. Skipping disks / networks / components
        // keeps each refresh under a millisecond on Linux.
        let refresh_kind =
            RefreshKind::new().with_processes(ProcessRefreshKind::new().with_cpu().with_memory());
        let mut system = System::new_with_specifics(refresh_kind);
        let pid = Pid::from_u32(std::process::id());
        // Seed the process snapshot so the FIRST refresh has a
        // baseline to delta against (CPU % needs two samples).
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        Self {
            system,
            pid,
            last_refresh: None,
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
        state.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[state.pid]),
            true,
            ProcessRefreshKind::new().with_cpu().with_memory(),
        );
        state.last_refresh = Some(Instant::now());

        if let Some(proc) = state.system.process(state.pid) {
            let cpu_percent = proc.cpu_usage();
            // sysinfo returns memory in bytes (0.32+).
            let ram_bytes = proc.memory();
            let ram_rss_mb = (ram_bytes / (1024 * 1024)) as u32;
            if let Some(stats) = resources.get_mut::<EditorPerfStats>() {
                stats.cpu_percent = cpu_percent;
                stats.ram_rss_mb = ram_rss_mb;
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
}
