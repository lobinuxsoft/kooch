use super::*;

/// Builds a result carrying a timestamp range in seconds.
fn query(
    label: &str,
    start: f64,
    end: f64,
    nested: Vec<GpuTimerQueryResult>,
) -> GpuTimerQueryResult {
    GpuTimerQueryResult {
        label: label.to_owned(),
        pid: std::process::id(),
        tid: std::thread::current().id(),
        time: Some(start..end),
        nested_queries: nested,
    }
}

#[test]
fn no_timestamps_report_nothing() {
    let untimed = GpuTimerQueryResult {
        label: "raster".to_owned(),
        pid: std::process::id(),
        tid: std::thread::current().id(),
        time: None,
        nested_queries: Vec::new(),
    };
    assert!(clock_offset(&[untimed]).is_none());
    assert!(clock_offset(&[]).is_none());
}

/// The whole point of the offset: a raw GPU timestamp lands an
/// undefined distance from puffin's clock, and the viewer would draw a
/// frame stretched across the gap. Verified by shifting a batch whose
/// raw values sit ~317 years away from any wall clock.
#[test]
fn the_batch_ends_at_now() {
    let far_away = 1e10;
    let offset = clock_offset(&[query("raster", far_away, far_away + 0.004, vec![])])
        .expect("a timed scope has an offset");
    let end_on_puffin_clock = secs_to_ns(far_away + 0.004) + offset;
    let drift = (end_on_puffin_clock - puffin::now_ns()).abs();
    assert!(
        drift < 1_000_000,
        "batch end is {drift} ns off puffin's now"
    );
}

/// The deepest scope wins, not the last one read: without recursing
/// into children the offset would be computed from a parent that
/// closes before a child the profiler nested inside it.
#[test]
fn nested_ends_reach_the_offset() {
    let deep = query("frame", 0.0, 1.0, vec![query("shade", 0.1, 9.0, vec![])]);
    let offset = clock_offset(&[deep]).expect("timed");
    let latest = secs_to_ns(9.0) + offset;
    assert!((latest - puffin::now_ns()).abs() < 1_000_000);
}

/// Durations survive the translation — the offset is what moves, and
/// a pass that took 4 ms still reads as 4 ms.
#[test]
fn durations_are_preserved() {
    let results = [query("shadows", 100.0, 100.004, vec![])];
    let offset = clock_offset(&results).expect("timed");
    let time = results[0].time.as_ref().unwrap();
    let start = secs_to_ns(time.start) + offset;
    let end = secs_to_ns(time.end) + offset;
    assert_eq!(end - start, 4_000_000);
}
