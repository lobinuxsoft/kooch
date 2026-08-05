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
//! opportunistically in [`kooch_core::gpu::GpuContext::new`]; this resource
//! mirrors the runtime decision so the meshlet render stage can pick the
//! right format without re-querying `Device::features()` every call.

use wgpu::{Device, Features};

/// Triangle-id slot width in bits (matches Bevy). `DEFAULT_MAX_TRIANGLES`
/// is 124 in kooch, so 7 bits (range 0..128) is sufficient and
/// preserves bit-compatibility with Bevy's meshlet pipeline.
pub const TRI_ID_BITS: u32 = 7;

/// Bit mask for the triangle-id slot (low 7 bits of the packed-ids u32).
pub const TRI_ID_MASK: u32 = (1 << TRI_ID_BITS) - 1;

/// Cluster-id slot width in bits. The remaining high bits of the packed-ids
/// u32 above the triangle-id slot.
pub const CLUSTER_ID_BITS: u32 = 32 - TRI_ID_BITS;

/// Maximum cluster id representable by the pack format (≈ 33M meshlets).
pub const MAX_CLUSTER_ID: u32 = (1u32 << CLUSTER_ID_BITS) - 1;

/// Packs a fragment's reversed-Z depth + cluster id + in-meshlet triangle id
/// into a single u64 visibility-buffer entry.
///
/// Layout (high → low):
/// - bits `[63:32]` — `depth.to_bits()`. Under reversed-Z (NDC depth in
///   `[0, 1]`, 1.0 closest), the f32 bit pattern is monotonically ordered
///   for non-negative finite values, so `textureAtomicMax` over the u64
///   selects the closest fragment per pixel atomically.
/// - bits `[31:7]`  — `cluster_id` (25 bits, up to [`MAX_CLUSTER_ID`]).
/// - bits `[6:0]`   — `tri_id` (7 bits, up to 127). Mirrors
///   `DEFAULT_MAX_TRIANGLES` and matches Bevy's meshlet layout.
///
/// Mirrors Bevy's [hardware][1] / [software][2] raster vbuf write.
///
/// [1]: https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/meshlet/visibility_buffer_hardware_raster.wgsl
/// [2]: https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/meshlet/visibility_buffer_software_raster.wgsl
#[inline]
pub fn pack_visibility(depth: f32, cluster_id: u32, tri_id: u32) -> u64 {
    debug_assert!(
        tri_id <= TRI_ID_MASK,
        "tri_id {tri_id} exceeds {TRI_ID_BITS}-bit slot"
    );
    debug_assert!(
        cluster_id <= MAX_CLUSTER_ID,
        "cluster_id {cluster_id} exceeds {CLUSTER_ID_BITS}-bit slot"
    );
    let depth_bits = depth.to_bits() as u64;
    let packed_ids = ((cluster_id << TRI_ID_BITS) | (tri_id & TRI_ID_MASK)) as u64;
    (depth_bits << 32) | packed_ids
}

/// Inverse of [`pack_visibility`]. Returns `(depth, cluster_id, tri_id)`.
#[inline]
pub fn unpack_visibility(packed: u64) -> (f32, u32, u32) {
    let depth_bits = (packed >> 32) as u32;
    let packed_ids = packed as u32;
    let depth = f32::from_bits(depth_bits);
    let cluster_id = packed_ids >> TRI_ID_BITS;
    let tri_id = packed_ids & TRI_ID_MASK;
    (depth, cluster_id, tri_id)
}

/// Runtime support flag for the atomic R64 visibility buffer path.
///
/// Inserted into [`Resources`](kooch_core::resource::Resources) at render plugin
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
            tracing::info!("Vbuf64Support: atomic R64 path active (Nanite-style winner-takes-all)");
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
/// [`kooch_core::gpu::vbuf64_features`]; duplicated here to avoid a hard
/// dependency from `kooch_render` on `kooch_core::gpu` for this single helper.
///
/// `TEXTURE_ATOMIC` is the gate on `StorageTextureAccess::Atomic` in
/// wgpu 29 (the validation error message historically said
/// `TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES`, which is misleading —
/// the real check is `TEXTURE_ATOMIC`). `TEXTURE_INT64_ATOMIC` adds the
/// R64 format on top. `SHADER_INT64` enables `u64` in the shader.
/// `SHADER_INT64_ATOMIC_MIN_MAX` enables `textureAtomicMax(u64)`.
fn required_features() -> Features {
    Features::TEXTURE_ATOMIC
        | Features::TEXTURE_INT64_ATOMIC
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
    fn required_bundle_is_four_flags() {
        let bundle = required_features();
        assert!(bundle.contains(Features::TEXTURE_ATOMIC));
        assert!(bundle.contains(Features::TEXTURE_INT64_ATOMIC));
        assert!(bundle.contains(Features::SHADER_INT64));
        assert!(bundle.contains(Features::SHADER_INT64_ATOMIC_MIN_MAX));
    }

    #[test]
    fn pack_unpack_round_trips_mid_range() {
        let cases = [
            (0.5_f32, 12_345, 7),
            (0.123_f32, 0, 0),
            (0.999_f32, MAX_CLUSTER_ID, TRI_ID_MASK),
        ];
        for (d, c, t) in cases {
            let packed = pack_visibility(d, c, t);
            let (du, cu, tu) = unpack_visibility(packed);
            assert_eq!(du.to_bits(), d.to_bits(), "depth round-trip {d}");
            assert_eq!(cu, c, "cluster_id round-trip");
            assert_eq!(tu, t, "tri_id round-trip");
        }
    }

    #[test]
    fn pack_unpack_round_trips_reversed_z_extremes() {
        for depth in [0.0_f32, 1.0_f32, f32::MIN_POSITIVE, 1.0 - f32::EPSILON] {
            let packed = pack_visibility(depth, 17, 3);
            let (du, cu, tu) = unpack_visibility(packed);
            assert_eq!(du.to_bits(), depth.to_bits(), "depth {depth}");
            assert_eq!(cu, 17);
            assert_eq!(tu, 3);
        }
    }

    #[test]
    fn closer_reversed_z_depth_yields_larger_packed() {
        // Reversed-Z: closer fragment has the *higher* depth value, so the
        // packed u64 must be greater for the closer fragment. This is the
        // load-bearing invariant for `textureAtomicMax` to act as
        // winner-takes-all.
        let near = pack_visibility(0.95, 10, 0);
        let far = pack_visibility(0.20, 10, 0);
        assert!(near > far, "expected near > far ({near} vs {far})");
    }

    #[test]
    fn equal_depth_higher_cluster_id_wins_atomicmax() {
        // Bevy's tie-break: at equal depth, the fragment with the larger
        // packed_ids value wins under atomicMax. Document the behaviour so
        // the integration test for coplanar meshlets asserts the right
        // direction (larger cluster_id, not smaller).
        let lhs = pack_visibility(0.5, 100, 0);
        let rhs = pack_visibility(0.5, 99, 0);
        assert!(lhs > rhs, "tie-break: larger cluster_id wins");
    }

    #[test]
    fn default_max_triangles_fits_tri_id_slot() {
        use crate::meshlet::DEFAULT_MAX_TRIANGLES;
        assert!(
            DEFAULT_MAX_TRIANGLES as u32 <= TRI_ID_MASK + 1,
            "DEFAULT_MAX_TRIANGLES ({DEFAULT_MAX_TRIANGLES}) overflows {TRI_ID_BITS}-bit tri_id slot"
        );
    }
}
