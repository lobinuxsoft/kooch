//! Atomic R64 visibility buffer support detection (#493).
//!
//! Bevy's meshlet pipeline writes packed `(depth_bits << 32 | cluster_id << 7
//! | tri_id)` to an `R64Uint` storage texture via `textureAtomicMax`. Combined
//! with reversed-Z this turns the visibility buffer into a winner-takes-all
//! atomic — the closest fragment wins per pixel deterministically, eliminating
//! the z-fighting that the legacy `R32Uint` non-atomic path exhibits between
//! coplanar meshlets.
//!
//! The wgpu features that gate this path (`TEXTURE_INT64_ATOMIC`,
//! `SHADER_INT64`, `SHADER_INT64_ATOMIC_MIN_MAX`) are requested
//! opportunistically in [`ome_core::gpu::GpuContext::new`]; this resource
//! mirrors the runtime decision so the meshlet render stage can pick the
//! right format without re-querying `Device::features()` every call.

use wgpu::{Device, Features};

/// Runtime support flag for the atomic R64 visibility buffer path.
///
/// Inserted into [`Resources`](ome_core::resource::Resources) at render plugin
/// startup. Consumers (e.g. the meshlet render stage) read it once when
/// allocating the visibility buffer texture and bind groups.
#[derive(Debug, Clone, Copy)]
pub struct Vbuf64Support {
    supported: bool,
}

impl Vbuf64Support {
    /// Probes the device feature set to decide whether the atomic R64 vbuf
    /// path is available, logging the active path at info / warn level.
    pub fn detect(device: &Device) -> Self {
        let needed = required_features();
        let supported = device.features().contains(needed);
        if supported {
            tracing::info!(
                "Vbuf64Support: atomic R64 path active (Nanite-style winner-takes-all)"
            );
        } else {
            let missing = needed - device.features();
            tracing::warn!(
                ?missing,
                "Vbuf64Support: R32Uint fallback active (coplanar meshlets may z-fight; \
                 device missing one or more of TEXTURE_INT64_ATOMIC / SHADER_INT64 / \
                 SHADER_INT64_ATOMIC_MIN_MAX)"
            );
        }
        Self { supported }
    }

    /// Constructs a support flag with an explicit value. Intended for tests
    /// that do not own a `Device`.
    #[inline]
    pub const fn from_supported(supported: bool) -> Self {
        Self { supported }
    }

    /// Returns `true` when the device exposes the full `R64` atomic feature
    /// bundle and the meshlet render stage should take the atomic path.
    #[inline]
    pub const fn is_supported(&self) -> bool {
        self.supported
    }
}

/// Feature bundle required for the atomic R64 visibility buffer. Mirrors
/// [`ome_core::gpu::vbuf64_features`]; duplicated here to avoid a hard
/// dependency from `ome_render` on `ome_core::gpu` for this single helper.
fn required_features() -> Features {
    Features::TEXTURE_INT64_ATOMIC
        | Features::SHADER_INT64
        | Features::SHADER_INT64_ATOMIC_MIN_MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_supported_round_trips() {
        assert!(Vbuf64Support::from_supported(true).is_supported());
        assert!(!Vbuf64Support::from_supported(false).is_supported());
    }

    #[test]
    fn required_bundle_is_three_flags() {
        let bundle = required_features();
        assert!(bundle.contains(Features::TEXTURE_INT64_ATOMIC));
        assert!(bundle.contains(Features::SHADER_INT64));
        assert!(bundle.contains(Features::SHADER_INT64_ATOMIC_MIN_MAX));
    }
}
