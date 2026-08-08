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
    assert_eq!(s.breakdown, FrameBreakdown::default());
}

#[test]
fn vram_tracked_mb_truncates_correctly() {
    let mut s = EditorPerfStats::default();
    s.vram_tracked_bytes = 5 * 1024 * 1024 + 512; // 5 MB + a bit
    assert_eq!(s.vram_tracked_mb(), 5);
}
