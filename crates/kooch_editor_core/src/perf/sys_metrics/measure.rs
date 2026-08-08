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
