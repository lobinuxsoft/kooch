//! Change ticks (#1870): a process-wide clock that every component write stamps, and the tick the
//! running system last ran at, so it can ask for only what was written since.
//!
//! `u64`, not Bevy's `u32`: at a few hundred systems a frame a `u32` wraps in days and needs a
//! periodic clamp pass; a `u64` never does, for 8 bytes a row.

use std::sync::atomic::{AtomicU64, Ordering};

static CLOCK: AtomicU64 = AtomicU64::new(1);

/// The tick a write made now carries.
#[inline]
pub fn now() -> u64 {
    CLOCK.load(Ordering::Relaxed)
}

/// Starts a new tick and returns it.
#[inline]
pub fn advance() -> u64 {
    CLOCK.fetch_add(1, Ordering::Relaxed) + 1
}

/// The running system's window: what was written after `last_run` is new to it. Written by the
/// scheduler before every CPU system; `last_run` is 0 for a system that never ran, so it sees all.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SystemTick {
    pub last_run: u64,
    pub this_run: u64,
}

impl SystemTick {
    /// Whether a value stamped `tick` was written after this system last ran.
    #[inline]
    pub fn is_new(&self, tick: u64) -> bool {
        tick > self.last_run
    }
}
