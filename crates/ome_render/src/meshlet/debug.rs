//! Meshlet pipeline debug visualization modes (#451).
//!
//! The deferred shader branches on a single `u32` uniform pulled from
//! [`MeshletDebugMode`]. `Off` (the default) is the production normal
//! debug × material path; every other variant overrides the colour
//! output to expose a specific stage of the cull → vbuf → shade chain.
//!
//! Mode values are stable: the GPU shader pattern-matches on the raw
//! `u32` and exhaustive shader coverage relies on the discriminants
//! never being reordered. Add new variants at the end.

/// Debug-visualization selector for the meshlet pipeline. Lives in
/// [`Resources`](ome_core::resource::Resources) so the editor can
/// flip it per-frame without touching the render-stage struct.
#[repr(u32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum MeshletDebugMode {
    /// Production path — normal-debug shading × material base colour.
    #[default]
    Off = 0,
    /// `hash3(meshlet_id) → RGB`. Exposes the cluster boundaries.
    MeshletIds = 1,
    /// `hash3(instance_id) → RGB`. Exposes per-entity coverage.
    InstanceIds = 2,
    /// Heatmap of triangles drawn per pixel. Calibrates the LOD
    /// `target_error_pixels` knob — anything brighter than green is
    /// sub-pixel triangle territory.
    TriangleDensity = 3,
    /// Heatmap of visibility-buffer atomic writes per pixel. Hot spots
    /// indicate cluster overdraw the Hi-Z pass failed to reject.
    Overdraw = 4,
    /// Bright red on meshlets that the Hi-Z occlusion test discarded.
    HiZRejected = 5,
    /// Bright blue on meshlets that the backface-cone test discarded.
    BackfaceRejected = 6,
    /// Bright green on meshlets that survived every cull stage and
    /// reached the visibility buffer.
    CullPassthrough = 7,
}

impl MeshletDebugMode {
    /// Stable raw discriminant the deferred shader pattern-matches on.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    /// Modes whose shader implementation is wired and visually
    /// validated. The editor's debug-view dropdown iterates this so
    /// users never select a mode that silently falls back to `Off`.
    /// Extend as new modes ship per-commit.
    pub fn all_implemented() -> &'static [Self] {
        &[Self::Off, Self::MeshletIds, Self::InstanceIds]
    }

    /// Human-readable label for the editor dropdown / tooltips.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::MeshletIds => "Meshlet IDs",
            Self::InstanceIds => "Instance IDs",
            Self::TriangleDensity => "Triangle Density",
            Self::Overdraw => "Overdraw",
            Self::HiZRejected => "Hi-Z Rejected",
            Self::BackfaceRejected => "Backface Rejected",
            Self::CullPassthrough => "Cull Passthrough",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_is_zero() {
        assert_eq!(MeshletDebugMode::Off.as_u32(), 0);
        assert_eq!(MeshletDebugMode::default(), MeshletDebugMode::Off);
    }

    #[test]
    fn discriminants_are_stable() {
        // GPU shader assumes these exact values. Reordering breaks
        // every active debug mode silently — flip this test first.
        assert_eq!(MeshletDebugMode::Off.as_u32(), 0);
        assert_eq!(MeshletDebugMode::MeshletIds.as_u32(), 1);
        assert_eq!(MeshletDebugMode::InstanceIds.as_u32(), 2);
        assert_eq!(MeshletDebugMode::TriangleDensity.as_u32(), 3);
        assert_eq!(MeshletDebugMode::Overdraw.as_u32(), 4);
        assert_eq!(MeshletDebugMode::HiZRejected.as_u32(), 5);
        assert_eq!(MeshletDebugMode::BackfaceRejected.as_u32(), 6);
        assert_eq!(MeshletDebugMode::CullPassthrough.as_u32(), 7);
    }
}
