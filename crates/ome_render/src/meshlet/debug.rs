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

use super::caps::MeshletDebugCaps;

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
    /// Force-render ONLY meshlets at LOD 0 (the highest-detail
    /// chain entry, `lod_error == 0`). Bypasses the normal selector
    /// so the artist can inspect what the finest-LOD geometry looks
    /// like in isolation, free of any chain-descent overlap.
    /// Pairs with `OnlyRoots` for visual sanity-checking the
    /// LOD chain's two extremes.
    OnlyLod0 = 8,
    /// Force-render ONLY meshlets that are roots in the LOD DAG
    /// (`parent_meshlet_index == MESHLET_ROOT_PARENT`). Shows the
    /// coarsest-available representation of each registered mesh
    /// in isolation. Useful for distinguishing real LOD descent
    /// (transition between LOD 0 and roots as distance grows) from
    /// chain-construction failures (where everything is a root and
    /// the distance threshold has nothing to descend into).
    OnlyRoots = 9,
    /// Bright yellow on meshlets that the frustum test discarded.
    /// Frustum culls coherent groups (entire object behind / off-screen),
    /// so the overlay is the canonical way to spot per-cluster bounds
    /// that disagree with the object's macro AABB — a common artifact
    /// of stale build-time bounds after a mesh edit.
    FrustumRejected = 10,
}

/// Runtime knob for the cull / LOD selector. Lives as a
/// [`Resource`](ome_core::resource::Resources) so the editor can
/// adjust it in flight without rebuilding the meshlet stage.
///
/// `target_error_pixels` is the boundary the per-meshlet selector
/// compares against: a meshlet is picked when its own pixel-projected
/// `lod_error` falls under the target AND its parent's exceeds it.
/// Lower values keep more detail at any given distance; higher values
/// drop to coarser parents earlier.
#[derive(Copy, Clone, Debug)]
pub struct MeshletLodSettings {
    pub target_error_pixels: f32,
}

impl Default for MeshletLodSettings {
    fn default() -> Self {
        Self {
            target_error_pixels: 1.0,
        }
    }
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
        &[
            Self::Off,
            Self::MeshletIds,
            Self::InstanceIds,
            Self::TriangleDensity,
            Self::Overdraw,
            Self::CullPassthrough,
            Self::OnlyLod0,
            Self::OnlyRoots,
        ]
    }

    /// Reject-reason code the cull shader writes when this mode is
    /// active and `CullParams.debug_active != 0`. Mirrors the
    /// `REJECT_REASON_*` constants in `meshlet_cull/atomic.wgsl`.
    /// `None` for non-reject modes — the orchestrator uses the
    /// `Some/None` split to gate both the cull-side `debug_active`
    /// flag and the overlay dispatch.
    #[inline]
    pub const fn reject_reason_code(self) -> Option<u32> {
        match self {
            Self::FrustumRejected => Some(2),
            Self::BackfaceRejected => Some(3),
            Self::HiZRejected => Some(4),
            _ => None,
        }
    }

    /// `true` when the mode's pipeline writes to an R32Uint atomic
    /// storage texture (triangle-density accumulator, overdraw
    /// accumulator, reject-reason buffer). Those branches require
    /// `wgpu::Features::TEXTURE_ATOMIC`; on adapters without it the
    /// editor dropdown filter hides them.
    #[inline]
    pub const fn needs_texture_atomic(self) -> bool {
        matches!(
            self,
            Self::TriangleDensity
                | Self::Overdraw
                | Self::HiZRejected
                | Self::BackfaceRejected
                | Self::FrustumRejected,
        )
    }

    /// `true` when the mode can be selected on the current device.
    /// Filters out modes whose pipeline depends on a feature the
    /// adapter does not expose (today: `TEXTURE_ATOMIC`).
    #[inline]
    pub const fn is_available_with_caps(self, caps: &MeshletDebugCaps) -> bool {
        if self.needs_texture_atomic() {
            caps.supports_texture_atomic()
        } else {
            true
        }
    }

    /// Capability-aware dropdown list. Returns every mode currently
    /// wired in [`Self::all_implemented`], minus those the device
    /// cannot run. The editor's debug-view combobox iterates this
    /// so the user never selects a mode that would later fail
    /// pipeline validation.
    pub fn all_available_with_caps(caps: &MeshletDebugCaps) -> Vec<Self> {
        Self::all_implemented()
            .iter()
            .copied()
            .filter(|m| m.is_available_with_caps(caps))
            .collect()
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
            Self::OnlyLod0 => "Only LOD 0",
            Self::OnlyRoots => "Only Roots",
            Self::FrustumRejected => "Frustum Rejected",
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
    fn needs_texture_atomic_covers_advanced_modes() {
        assert!(MeshletDebugMode::TriangleDensity.needs_texture_atomic());
        assert!(MeshletDebugMode::Overdraw.needs_texture_atomic());
        assert!(MeshletDebugMode::HiZRejected.needs_texture_atomic());
        assert!(MeshletDebugMode::BackfaceRejected.needs_texture_atomic());
        assert!(MeshletDebugMode::FrustumRejected.needs_texture_atomic());
        // Baseline-safe modes never lift the atomic feature gate.
        assert!(!MeshletDebugMode::Off.needs_texture_atomic());
        assert!(!MeshletDebugMode::MeshletIds.needs_texture_atomic());
        assert!(!MeshletDebugMode::InstanceIds.needs_texture_atomic());
        assert!(!MeshletDebugMode::CullPassthrough.needs_texture_atomic());
        assert!(!MeshletDebugMode::OnlyLod0.needs_texture_atomic());
        assert!(!MeshletDebugMode::OnlyRoots.needs_texture_atomic());
    }

    #[test]
    fn all_available_with_caps_filters_atomic_modes() {
        // Conservative caps (texture_atomic missing): only the
        // baseline-safe subset of `all_implemented()` survives.
        let no_atomic = MeshletDebugCaps::from_flags(false);
        let filtered = MeshletDebugMode::all_available_with_caps(&no_atomic);
        for mode in &filtered {
            assert!(
                !mode.needs_texture_atomic(),
                "{mode:?} leaked through the filter without atomic support",
            );
        }
        // With atomic support, the filter is identity over `all_implemented`.
        let with_atomic = MeshletDebugCaps::from_flags(true);
        let unfiltered = MeshletDebugMode::all_available_with_caps(&with_atomic);
        assert_eq!(unfiltered.len(), MeshletDebugMode::all_implemented().len());
    }

    #[test]
    fn reject_reason_code_tracks_cull_shader_constants() {
        // `REJECT_REASON_*` in meshlet_cull/atomic.wgsl pin these.
        // Reordering or renumbering breaks the overlay's match.
        assert_eq!(MeshletDebugMode::FrustumRejected.reject_reason_code(), Some(2));
        assert_eq!(MeshletDebugMode::BackfaceRejected.reject_reason_code(), Some(3));
        assert_eq!(MeshletDebugMode::HiZRejected.reject_reason_code(), Some(4));
        // Non-reject modes never write into reject_reasons[] — the
        // orchestrator must NOT lift `debug_active` for them.
        assert!(MeshletDebugMode::Off.reject_reason_code().is_none());
        assert!(MeshletDebugMode::TriangleDensity.reject_reason_code().is_none());
        assert!(MeshletDebugMode::Overdraw.reject_reason_code().is_none());
        assert!(MeshletDebugMode::CullPassthrough.reject_reason_code().is_none());
        assert!(MeshletDebugMode::OnlyLod0.reject_reason_code().is_none());
        assert!(MeshletDebugMode::OnlyRoots.reject_reason_code().is_none());
        assert!(MeshletDebugMode::MeshletIds.reject_reason_code().is_none());
        assert!(MeshletDebugMode::InstanceIds.reject_reason_code().is_none());
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
        assert_eq!(MeshletDebugMode::OnlyLod0.as_u32(), 8);
        assert_eq!(MeshletDebugMode::OnlyRoots.as_u32(), 9);
        assert_eq!(MeshletDebugMode::FrustumRejected.as_u32(), 10);
    }
}
