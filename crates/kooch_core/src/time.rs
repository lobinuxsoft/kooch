//! Time management for the game loop.
//!
//! Implements "Fix Your Timestep" pattern with separate delta times for:
//! - Variable-rate rendering (delta)
//! - Fixed-rate physics (fixed_delta)

use std::time::{Duration, Instant};

/// Time resource containing frame timing information.
///
/// # Fixed Timestep
/// Physics updates run at a fixed rate (default 60Hz) regardless of frame rate.
/// The accumulator tracks how much "physics time" has built up, and the game
/// loop runs physics updates until the accumulator is drained.
///
/// # Render Interpolation
/// `render_alpha` provides the interpolation factor for smooth rendering
/// between physics states: `render_alpha = accumulator / fixed_delta`
#[derive(Debug, Clone)]
pub struct Time {
    /// When the application started.
    startup: Instant,
    /// When the current frame started.
    frame_start: Instant,
    /// When the previous frame started.
    last_frame_start: Instant,

    /// Time elapsed since the previous frame (variable).
    delta: Duration,
    /// Total time elapsed since startup.
    elapsed: Duration,

    /// Fixed timestep for physics (default: 1/60 second).
    fixed_delta: Duration,
    /// Accumulated time for fixed updates.
    accumulator: Duration,
    /// Maximum accumulated time to prevent spiral of death.
    max_accumulator: Duration,

    /// Interpolation factor for rendering between physics states [0.0, 1.0].
    render_alpha: f32,

    /// Frame counter.
    frame_count: u64,
    /// Fixed update counter.
    fixed_count: u64,
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

impl Time {
    /// Creates a new Time resource with default settings.
    ///
    /// Default fixed timestep: 60 Hz (16.67ms)
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            startup: now,
            frame_start: now,
            last_frame_start: now,
            delta: Duration::ZERO,
            elapsed: Duration::ZERO,
            fixed_delta: Duration::from_secs_f64(1.0 / 60.0),
            accumulator: Duration::ZERO,
            max_accumulator: Duration::from_secs_f64(0.25), // Max 250ms accumulator
            render_alpha: 1.0,
            frame_count: 0,
            fixed_count: 0,
        }
    }

    /// Sets the fixed timestep rate in Hz.
    ///
    /// # Example
    /// ```
    /// use kooch_core::time::Time;
    ///
    /// let mut time = Time::new();
    /// time.set_fixed_hz(120.0); // 120 Hz physics
    /// ```
    pub fn set_fixed_hz(&mut self, hz: f64) {
        self.fixed_delta = Duration::from_secs_f64(1.0 / hz);
    }

    /// Sets the fixed timestep duration directly.
    pub fn set_fixed_delta(&mut self, delta: Duration) {
        self.fixed_delta = delta;
    }

    /// Updates timing for a new frame.
    ///
    /// Returns the number of fixed timestep updates that should run this frame.
    pub fn update(&mut self) -> u32 {
        let now = Instant::now();
        self.last_frame_start = self.frame_start;
        self.frame_start = now;

        self.delta = self.frame_start.duration_since(self.last_frame_start);
        self.elapsed = self.frame_start.duration_since(self.startup);
        self.frame_count += 1;

        // Accumulate time for fixed updates
        self.accumulator += self.delta;

        // Clamp to prevent spiral of death
        if self.accumulator > self.max_accumulator {
            self.accumulator = self.max_accumulator;
        }

        // Calculate how many fixed steps to run
        let mut fixed_updates = 0u32;
        while self.accumulator >= self.fixed_delta {
            self.accumulator -= self.fixed_delta;
            self.fixed_count += 1;
            fixed_updates += 1;
        }

        // Calculate render interpolation alpha
        self.render_alpha = self.accumulator.as_secs_f32() / self.fixed_delta.as_secs_f32();

        fixed_updates
    }

    /// Advances time for testing without using real time.
    ///
    /// # Example
    /// ```
    /// use kooch_core::time::Time;
    /// use std::time::Duration;
    ///
    /// let mut time = Time::new();
    /// let updates = time.advance(Duration::from_millis(100));
    /// // 100ms / 16.67ms ≈ 5-6 updates (depends on float precision)
    /// assert!(updates >= 5 && updates <= 6);
    /// ```
    pub fn advance(&mut self, duration: Duration) -> u32 {
        self.delta = duration;
        self.elapsed += duration;
        self.frame_count += 1;

        self.accumulator += self.delta;

        if self.accumulator > self.max_accumulator {
            self.accumulator = self.max_accumulator;
        }

        let mut fixed_updates = 0u32;
        while self.accumulator >= self.fixed_delta {
            self.accumulator -= self.fixed_delta;
            self.fixed_count += 1;
            fixed_updates += 1;
        }

        self.render_alpha = self.accumulator.as_secs_f32() / self.fixed_delta.as_secs_f32();

        fixed_updates
    }

    /// Time elapsed since the previous frame.
    #[inline]
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// Time elapsed since the previous frame in seconds.
    #[inline]
    pub fn delta_secs(&self) -> f32 {
        self.delta.as_secs_f32()
    }

    /// Time elapsed since the previous frame in seconds (f64).
    #[inline]
    pub fn delta_secs_f64(&self) -> f64 {
        self.delta.as_secs_f64()
    }

    /// Total time elapsed since application startup.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Total time elapsed since startup in seconds.
    #[inline]
    pub fn elapsed_secs(&self) -> f32 {
        self.elapsed.as_secs_f32()
    }

    /// Total time elapsed since startup in seconds (f64).
    #[inline]
    pub fn elapsed_secs_f64(&self) -> f64 {
        self.elapsed.as_secs_f64()
    }

    /// Fixed timestep duration for physics updates.
    #[inline]
    pub fn fixed_delta(&self) -> Duration {
        self.fixed_delta
    }

    /// Fixed timestep in seconds.
    #[inline]
    pub fn fixed_delta_secs(&self) -> f32 {
        self.fixed_delta.as_secs_f32()
    }

    /// Interpolation factor for rendering between physics states.
    ///
    /// Use this to interpolate visual positions between the current
    /// and previous physics states for smooth rendering.
    ///
    /// Value range: [0.0, 1.0)
    #[inline]
    pub fn render_alpha(&self) -> f32 {
        self.render_alpha
    }

    /// Number of frames rendered since startup.
    #[inline]
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// When [`update`](Self::update) last ran — the start of this frame.
    ///
    /// What a late-stage system needs to time the frame's *work*:
    /// `time.frame_start().elapsed()` at `Stage::Last` excludes whatever
    /// the loop then spends waiting, which [`delta`](Self::delta) does
    /// not.
    #[inline]
    pub fn frame_start(&self) -> Instant {
        self.frame_start
    }

    /// How long until the next fixed step comes due.
    ///
    /// Zero when a step is already owed. A loop with nothing to draw has
    /// no reason to wake before this: running faster only re-reads the
    /// clock and finds no work (#656).
    pub fn until_next_fixed_step(&self) -> Duration {
        self.fixed_delta.saturating_sub(self.accumulator)
    }

    /// Number of fixed timestep updates since startup.
    #[inline]
    pub fn fixed_count(&self) -> u64 {
        self.fixed_count
    }

    /// Current fixed update rate in Hz.
    #[inline]
    pub fn fixed_hz(&self) -> f64 {
        1.0 / self.fixed_delta.as_secs_f64()
    }
}

#[cfg(test)]
mod tests;
