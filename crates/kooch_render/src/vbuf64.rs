//! Atomic R64 visibility buffer support detection (#493).

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

/// Packs a fragment's reversed-Z depth + cluster id + in-meshlet triangle id into a single u64
/// visibility-buffer entry.
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

/// Feature bundle required for the atomic R64 visibility buffer.
fn required_features() -> Features {
    kooch_core::gpu::vbuf64_features()
}

#[cfg(test)]
mod tests;
