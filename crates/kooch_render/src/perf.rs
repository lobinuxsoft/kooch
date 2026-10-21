//! Cross-module engine perf counters (#463.5).

use std::sync::atomic::{AtomicU64, Ordering};

/// What the engine holds on the device, published once a frame by whoever can ask the owners.
#[derive(Default, Debug)]
pub struct EngineVramTracker {
    bytes: AtomicU64,
}

impl EngineVramTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publishes what the engine holds, as measured this frame.
    ///
    /// 🔴 A total, not a counter. It used to be `add` / `sub` called from nine places, and a site
    /// that reported a growth but forgot the shrink could only ever climb (#1586). Asking the
    /// owners is the only arrangement where forgetting is impossible.
    pub fn publish(&self, bytes: u64) {
        self.bytes.store(bytes, Ordering::Relaxed);
    }

    /// What the engine held when `publish` was last called — what the HUD reads.
    pub fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests;
