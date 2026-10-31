//! Time management for the game loop.

use std::time::{Duration, Instant};

/// Time resource containing frame timing information.
#[derive(Debug, Clone)]
pub struct Time {
    /// When the current frame started.
    frame_start: Instant,
    /// When the previous frame started.
    last_frame_start: Instant,

    /// Game time since the previous frame: real time times `scale`.
    delta: Duration,
    /// Game time since startup: the sum of every scaled delta.
    elapsed: Duration,
    /// Real time since the previous frame, for what a pause must not stop (UI, editor).
    unscaled_delta: Duration,
    /// Real time since startup.
    unscaled_elapsed: Duration,
    /// How fast game time runs: 0 pauses, 1 is real time.
    scale: f32,

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
            frame_start: now,
            last_frame_start: now,
            delta: Duration::ZERO,
            elapsed: Duration::ZERO,
            unscaled_delta: Duration::ZERO,
            unscaled_elapsed: Duration::ZERO,
            scale: 1.0,
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

    /// Sets how fast game time runs: 0 pauses gameplay, 0.5 is slow motion. Negative or NaN reads
    /// as 0.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = if scale > 0.0 { scale } else { 0.0 };
    }

    /// How fast game time runs; 1 is real time.
    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Updates timing for a new frame.
    ///
    /// Returns the number of fixed timestep updates that should run this frame.
    pub fn update(&mut self) -> u32 {
        let now = Instant::now();
        self.last_frame_start = self.frame_start;
        self.frame_start = now;
        self.tick(self.frame_start.duration_since(self.last_frame_start))
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
        self.tick(duration)
    }

    /// The scale feeds the accumulator, not the step: slow motion runs fewer steps of the same
    /// size, so the simulation answers the same at any scale.
    fn tick(&mut self, real: Duration) -> u32 {
        self.unscaled_delta = real;
        self.unscaled_elapsed += real;
        self.delta = real.mul_f32(self.scale);
        self.elapsed += self.delta;
        self.frame_count += 1;

        self.accumulator += self.delta;
        // Clamp to prevent spiral of death
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

    /// Game time since the previous frame: real time times [`scale`](Self::scale).
    #[inline]
    pub fn delta(&self) -> Duration {
        self.delta
    }

    /// Time elapsed since the previous frame in seconds.
    #[inline]
    pub fn delta_secs(&self) -> f32 {
        self.delta.as_secs_f32()
    }

    /// Real time since the previous frame: what keeps running while gameplay is paused.
    #[inline]
    pub fn unscaled_delta(&self) -> Duration {
        self.unscaled_delta
    }

    /// Real time since the previous frame in seconds.
    #[inline]
    pub fn unscaled_delta_secs(&self) -> f32 {
        self.unscaled_delta.as_secs_f32()
    }

    /// Real time since startup.
    #[inline]
    pub fn unscaled_elapsed(&self) -> Duration {
        self.unscaled_elapsed
    }

    /// Game time since startup: it stands still while paused.
    #[inline]
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Total time elapsed since startup in seconds.
    #[inline]
    pub fn elapsed_secs(&self) -> f32 {
        self.elapsed.as_secs_f32()
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
    #[inline]
    pub fn frame_start(&self) -> Instant {
        self.frame_start
    }

    /// Number of fixed timestep updates since startup.
    #[inline]
    pub fn fixed_count(&self) -> u64 {
        self.fixed_count
    }

    #[cfg(test)]
    /// Current fixed update rate in Hz.
    #[inline]
    pub fn fixed_hz(&self) -> f64 {
        1.0 / self.fixed_delta.as_secs_f64()
    }
}

#[cfg(test)]
mod tests;
