//! Cross-module engine perf counters (#463.5).
//!
//! Today: VRAM-tracked total. The engine instruments the buffer /
//! texture creations it owns (asset pool, render targets, etc.) so
//! the editor HUD can report a meaningful "engine VRAM footprint"
//! without a vendor-specific GPU memory query — those queries do not
//! exist as a portable Rust API today (Vulkan / D3D12 / Metal each
//! need their own native call).
//!
//! Future: this module is the natural home for any other
//! cross-module render-side counter (allocation churn, dispatch
//! count, etc.) that the editor's HUD or future telemetry exposes.
//!
//! ## Tracker semantics
//!
//! - `add` / `sub` are atomic relaxed — counters do not coordinate
//!   with anything else, only the most-recently-published value
//!   matters.
//! - The tracker is wrapped in `Arc` so multiple subsystems
//!   (MeshletRenderStage, MeshletPipeline, sky pass, gizmos…) can
//!   write into the same counter without passing it through every
//!   function signature.
//! - The HUD reads `bytes()` once per frame — no syncing needed.

use std::sync::atomic::{AtomicU64, Ordering};

/// Cumulative count of GPU buffer / texture bytes the engine has
/// allocated through wgpu and is currently holding live. Excludes
/// driver overhead, swap chain, and anything wgpu allocates
/// implicitly (uniform staging, descriptor heaps, etc.) — those
/// require per-backend queries the wgpu API does not expose
/// portably.
///
/// What IS counted today:
/// - GlobalMeshPool persistent vertex / index / meshlet / triangle
///   storage (the dominant footprint of a loaded scene)
/// - MeshletRenderStage's vbuf / depth / color render targets
///
/// What is NOT counted today (intentionally — out-of-scope for the
/// engine-tracked HUD field):
/// - MaterialPool / GPU bind-group descriptors (kilobyte-range)
/// - egui texture atlas / per-frame staging
/// - sky / gizmo small uniform buffers
/// - the GPU's actual driver overhead (swap chain, descriptor heaps,
///   command pools)
///
/// The number is informational — useful for "watch VRAM grow as I
/// load this asset", NOT for "am I about to OOM the GPU".
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
mod tests {
    use super::*;

    #[test]
    fn add_then_read() {
        let t = EngineVramTracker::new();
        t.add(1024);
        t.add(2048);
        assert_eq!(t.bytes(), 3072);
    }

    #[test]
    fn sub_saturates_at_zero() {
        let t = EngineVramTracker::new();
        t.add(100);
        t.sub(200);
        assert_eq!(t.bytes(), 0, "sub past zero must clamp, not wrap");
    }

    #[test]
    fn reset_zeroes_the_counter() {
        let t = EngineVramTracker::new();
        t.add(9999);
        t.reset();
        assert_eq!(t.bytes(), 0);
    }
}
