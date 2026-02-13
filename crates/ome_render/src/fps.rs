//! Lightweight frame-rate tracker.

use std::time::Instant;

/// Counts frames over a 1-second window and reports FPS.
pub struct FpsTracker {
    last_report: Instant,
    frame_count: u32,
    current_fps: f64,
}

impl Default for FpsTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FpsTracker {
    /// Creates a new tracker starting now.
    pub fn new() -> Self {
        Self {
            last_report: Instant::now(),
            frame_count: 0,
            current_fps: 0.0,
        }
    }

    /// Records a frame. Returns `Some(fps)` when a full second has elapsed.
    pub fn tick(&mut self) -> Option<f64> {
        self.frame_count += 1;
        let elapsed = self.last_report.elapsed().as_secs_f64();

        if elapsed >= 1.0 {
            self.current_fps = f64::from(self.frame_count) / elapsed;
            self.frame_count = 0;
            self.last_report = Instant::now();
            Some(self.current_fps)
        } else {
            None
        }
    }

    /// Returns the last computed FPS value.
    #[inline]
    pub fn fps(&self) -> f64 {
        self.current_fps
    }
}
