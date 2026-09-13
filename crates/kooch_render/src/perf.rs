//! Cross-module engine perf counters (#463.5).

use std::sync::atomic::{AtomicU64, Ordering};

/// Cumulative count of GPU buffer / texture bytes the engine has allocated through wgpu and is
/// currently holding live.
#[derive(Default, Debug)]
pub struct EngineVramTracker {
    bytes: AtomicU64,
}

impl EngineVramTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bumps the tracked total. Called when the engine creates a
    /// buffer / texture it intends to keep around.
    pub fn add(&self, n: u64) {
        self.bytes.fetch_add(n, Ordering::Relaxed);
    }

    /// Decrements the tracked total. Called when the engine releases
    /// (or grows-via-realloc + drops) a previously-tracked resource.
    pub fn sub(&self, n: u64) {
        // Saturating: the tracker never goes negative even if a
        // double-sub or counter drift sneaks in. The HUD is
        // informational — better to under-report than to wrap.
        let current = self.bytes.load(Ordering::Relaxed);
        let new_val = current.saturating_sub(n);
        self.bytes.store(new_val, Ordering::Relaxed);
    }

    /// Current cumulative byte count — what the HUD reads.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    /// Resets to zero. Hands every previously-counted byte back as
    /// "released" without invoking `sub` for each. Used when the
    /// engine tears down a level or the editor swaps projects.
    pub fn reset(&self) {
        self.bytes.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests;
