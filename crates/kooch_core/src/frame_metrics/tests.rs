use super::*;

#[test]
fn a_frame_of_sixteen_milliseconds_is_sixty_fps() {
    let mut metrics = FrameMetrics::default();
    // The first call only banks its work; the second publishes it.
    metrics.record(Duration::from_micros(16_667), Duration::from_millis(4));
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
    light.record(Duration::from_micros(16_667), Duration::from_millis(2));
    let mut heavy = FrameMetrics::default();
    heavy.record(Duration::from_micros(16_667), Duration::from_millis(8));
    heavy.record(Duration::from_micros(16_667), Duration::from_millis(8));

    assert_eq!(light.fps_instant, heavy.fps_instant);
    assert!(heavy.cpu_frame_ms > light.cpu_frame_ms * 3.0);
}

/// A stall shows in the instant reading and is diluted in the average.
#[test]
fn one_late_frame_does_not_swing_the_average() {
    let mut metrics = FrameMetrics::default();
    for _ in 0..AVERAGE_WINDOW + 1 {
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
    for _ in 0..AVERAGE_WINDOW * 3 + 1 {
        metrics.record(Duration::from_micros(16_667), Duration::from_millis(4));
    }
    assert_eq!(metrics.recent.len(), AVERAGE_WINDOW);
    assert!((metrics.fps_average - 60.0).abs() < 0.5);
}

#[test]
fn a_zero_length_frame_reports_no_measurement_rather_than_infinity() {
    let mut metrics = FrameMetrics::default();
    metrics.record(Duration::ZERO, Duration::ZERO);
    metrics.record(Duration::ZERO, Duration::ZERO);
    assert_eq!(metrics.fps_instant, 0.0);
    assert!(metrics.fps_average.is_finite());
}

/// The bug this alignment exists for, in the shape it was found in.
///
/// A game whose frame rate changes — someone turned the camera —
/// reported `394 fps, frame 2.53 ms, cpu 7.51 ms`: work that does not
/// fit inside the frame it is attributed to. The two numbers came from
/// different frames, and while the rate was steady they agreed by
/// coincidence.
#[test]
fn the_work_always_fits_inside_the_frame_it_is_reported_with() {
    let mut metrics = FrameMetrics::default();
    // A cheap frame followed by a short wall clock and heavy work —
    // reported together, the work does not fit in the frame.
    metrics.record(Duration::from_millis(10), Duration::from_millis(1));
    metrics.record(Duration::from_millis(2), Duration::from_millis(8));
    assert!(
        metrics.cpu_frame_ms <= metrics.frame_ms,
        "cpu {} ms cannot exceed frame {} ms",
        metrics.cpu_frame_ms,
        metrics.frame_ms,
    );
}

/// Specifically: the published work is the one banked a frame earlier,
/// not the one that just arrived.
#[test]
fn the_pair_describes_one_frame_not_two() {
    let mut metrics = FrameMetrics::default();
    metrics.record(Duration::from_millis(10), Duration::from_millis(6));
    metrics.record(Duration::from_millis(10), Duration::from_millis(1));

    assert!(
        (metrics.cpu_frame_ms - 6.0).abs() < 0.01,
        "publishes the frame whose duration it just learned, got {}",
        metrics.cpu_frame_ms,
    );
}

/// One frame in and there is nothing honest to say yet.
#[test]
fn the_first_frame_publishes_nothing() {
    let mut metrics = FrameMetrics::default();
    metrics.record(Duration::from_millis(1900), Duration::from_millis(50));
    assert_eq!(metrics.frame_ms, 0.0);
    assert_eq!(metrics.fps_instant, 0.0);
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
