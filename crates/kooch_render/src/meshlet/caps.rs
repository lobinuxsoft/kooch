//! Runtime capability probe for advanced meshlet debug modes (#454).

use wgpu::{Device, Features};

/// Capability snapshot consumed by `MeshletDebugMode::all_implemented_with_caps`
/// and the editor's debug-view dropdown. Constructed once at startup and
/// inserted into [`Resources`](kooch_core::resource::Resources).
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
                target: "kooch_render::meshlet::caps",
                "MeshletDebugCaps: TEXTURE_ATOMIC available; advanced debug modes enabled",
            );
        } else {
            tracing::warn!(
                target: "kooch_render::meshlet::caps",
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
mod tests;
