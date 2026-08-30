use super::*;

#[test]
fn first_frame_does_not_emit_fps() {
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    frame_timer_system(&mut resources);
    let stats = resources.get::<EditorPerfStats>().unwrap();
    assert_eq!(stats.fps_instant, 0.0, "first frame must leave FPS at 0");
    assert_eq!(stats.fps_avg, 0.0);
}

#[test]
fn second_frame_emits_finite_fps() {
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    // First call: just captures the timestamp.
    frame_timer_system(&mut resources);
    // Sleep long enough that the delta is measurable but well
    // under the 1-second outlier guard.
    std::thread::sleep(std::time::Duration::from_millis(10));
    frame_timer_system(&mut resources);
    let stats = resources.get::<EditorPerfStats>().unwrap();
    assert!(
        stats.fps_instant > 0.0,
        "FPS instant must be populated after the second frame"
    );
    assert!(stats.fps_instant.is_finite());
    assert!(stats.fps_avg > 0.0);
    assert!(stats.fps_avg.is_finite());
}

#[test]
fn record_cpu_frame_ms_writes_elapsed() {
    let mut resources = Resources::default();
    resources.insert(EditorPerfStats::default());
    let start = Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(2));
    record_cpu_frame_ms(&mut resources, start);
    let stats = resources.get::<EditorPerfStats>().unwrap();
    assert!(
        stats.cpu_frame_ms >= 2.0,
        "expected ≥ 2 ms, got {}",
        stats.cpu_frame_ms
    );
    assert!(
        stats.cpu_frame_ms < 1000.0,
        "elapsed read should not be wildly off"
    );
}

/// 🔴 The frame is the whole frame, not the render system.
///
/// This exists because the HUD reported 7.66 ms on a frame that took
/// 50.9, with forty of them in `remote_sync_system` — outside the span
/// `cpu_frame_ms` covers. Nothing was wrong with `cpu_frame_ms`; it was
/// the only number on screen.
#[test]
fn the_frame_outlives_the_render_system() {
    let mut resources = Resources::new();
    resources.insert(EditorPerfStats::default());

    frame_timer_system(&mut resources);
    std::thread::sleep(std::time::Duration::from_millis(20));
    frame_timer_system(&mut resources);
    record_cpu_frame_ms(&mut resources, Instant::now());

    let stats = *resources.get::<EditorPerfStats>().expect("stats");
    assert!(
        stats.frame_ms >= 19.0,
        "the frame lost the wall clock: {}",
        stats.frame_ms,
    );
    assert!(
        stats.frame_ms > stats.cpu_frame_ms,
        "a render system of {} cannot fill a frame of {}",
        stats.cpu_frame_ms,
        stats.frame_ms,
    );
    // The same average the frame rate is derived from, or the two
    // lines of the HUD disagree about the same frame.
    assert!(
        (1000.0 / stats.frame_ms - stats.fps_avg).abs() < 0.01,
        "{} ms and {} fps describe different frames",
        stats.frame_ms,
        stats.fps_avg,
    );
}
