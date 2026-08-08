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
