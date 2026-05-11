//! Runtime capability probe for advanced meshlet debug modes (#454).
//!
//! Mirrors the [`Vbuf64Support`](crate::vbuf64::Vbuf64Support) shape:
//! detect at engine startup, stash in [`Resources`](ome_core::resource::Resources),
//! query when deciding which debug modes the editor dropdown should
//! surface to the user. Keeping the probe in a dedicated resource (vs.
//! re-querying `Device::features()` per frame) costs one boolean of
//! memory and saves every consumer from duplicating the feature-bit
//! arithmetic.
//!
//! The advanced debug modes (`TriangleDensity`, `Overdraw`,
//! `FrustumRejected`, `BackfaceRejected`, `HiZRejected`) each rely on
//! `texture_storage_2d<r32uint, atomic>` to accumulate per-pixel
//! counters or reject reasons. wgpu gates that on the
//! [`Features::TEXTURE_ATOMIC`] flag; without it the modes have no
//! production-quality fallback and stay hidden from the dropdown.
//!
//! `TEXTURE_ATOMIC` is broadly available across the supported
//! baseline (RDNA 2 / Turing / Adreno X1 and newer). Pre-baseline
//! adapters (Pascal, RDNA 1, Adreno 690 and older) still run the
//! engine through the R32 vbuf path — they just lose access to the
//! atomic-counter-based debug views.
//!
//! Future flags (Hi-Z scene path active, mesh-shader native, …) ride
//! on the same resource so the editor only has to read one struct.

use wgpu::{Device, Features};

/// Capability snapshot consumed by `MeshletDebugMode::all_implemented_with_caps`
/// and the editor's debug-view dropdown. Constructed once at startup and
/// inserted into [`Resources`](ome_core::resource::Resources).
#[derive(Debug, Clone, Copy)]
pub struct MeshletDebugCaps {
    /// `true` when the device exposes [`Features::TEXTURE_ATOMIC`].
    /// Gates every advanced debug mode that needs an R32Uint atomic
    /// accumulator (TriangleDensity, Overdraw, reject overlays).
    texture_atomic: bool,
}

impl MeshletDebugCaps {
    /// Probes the device feature set and logs the result at info level
    /// so the active debug-mode subset is observable from a release log.
    pub fn detect(device: &Device) -> Self {
        let texture_atomic = device.features().contains(Features::TEXTURE_ATOMIC);
        if texture_atomic {
            tracing::info!(
                target: "ome_render::meshlet::caps",
                "MeshletDebugCaps: TEXTURE_ATOMIC available; advanced debug modes enabled",
            );
        } else {
            tracing::warn!(
                target: "ome_render::meshlet::caps",
                "MeshletDebugCaps: TEXTURE_ATOMIC missing; advanced debug modes hidden \
                 (engine baseline is RDNA 2 / Turing / Adreno X1)",
            );
        }
        Self { texture_atomic }
    }

    /// Constructs a snapshot with explicit values. Intended for tests
    /// that do not own a `Device`.
    #[inline]
    pub const fn from_flags(texture_atomic: bool) -> Self {
        Self { texture_atomic }
    }

    /// `true` when the advanced debug modes that depend on an R32Uint
    /// atomic storage texture (TriangleDensity, Overdraw, reject
    /// overlays) can be wired without breaking pipeline validation.
    #[inline]
    pub const fn supports_texture_atomic(&self) -> bool {
        self.texture_atomic
    }
}

impl Default for MeshletDebugCaps {
    /// Conservative default — no advanced features. Used in tests and
    /// any path that has not run [`Self::detect`] yet.
    fn default() -> Self {
        Self::from_flags(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_conservative() {
        let caps = MeshletDebugCaps::default();
        assert!(!caps.supports_texture_atomic());
    }

    #[test]
    fn from_flags_round_trips() {
        assert!(MeshletDebugCaps::from_flags(true).supports_texture_atomic());
        assert!(!MeshletDebugCaps::from_flags(false).supports_texture_atomic());
    }
}
