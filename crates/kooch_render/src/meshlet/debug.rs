//! Meshlet pipeline debug visualization modes (#451).
//!
//! The deferred shader branches on a single `u32` uniform pulled from
//! [`MeshletDebugMode`]. `Off` (the default) is the production path —
//! since #441 that means Inti's Cook-Torrance shading, not the
//! world-space normal painted as colour that used to ship as the
//! shading model. Every other variant overrides the colour output to
//! expose a specific stage of the cull → vbuf → shade chain.
//!
//! Mode values are stable: the GPU shader pattern-matches on the raw
//! `u32` and exhaustive shader coverage relies on the discriminants
//! never being reordered. Add new variants at the end.

use super::caps::MeshletDebugCaps;

/// Debug-visualization selector for the meshlet pipeline. Lives in
/// [`Resources`](kooch_core::resource::Resources) so the editor can
/// flip it per-frame without touching the render-stage struct.
#[repr(u32)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum MeshletDebugMode {
    /// Production path — Cook-Torrance shading driven by the scene's
    /// lights (#441). Before that landed, this was the normal-debug
    /// view, which is now [`Self::Normals`].
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
    /// World-space normal painted as colour, modulated by albedo.
    ///
    /// This was the engine's *shading model* until #441 — a debug view
    /// shipped as the production path, which is why a scene with lights
    /// and a scene without them rendered identically. It is a genuinely
    /// useful view of the geometry, so it survives here; it just stops
    /// being what you get by default.
    Normals = 11,
    /// What the shadow system sees, as colour (#476).
    ///
    /// A missing shadow is one of three things that look identical in a
    /// shaded frame — the cascade does not reach here, the occluder was
    /// culled out of the map, or the sampling is wrong — and they have
    /// different fixes. This separates them: hue is the cascade, bright
    /// means an occluder is recorded over the point, black means the
    /// point is inside no cascade volume at all, and magenta means
    /// nothing casts.
    ///
    /// It is also the view that makes cascade placement visible: the
    /// bands are the split distances, and a project whose shadow
    /// distance is far larger than its scene sees one colour everywhere
    /// near the camera and the rest wasted.
    ShadowCascades = 12,
    /// What the contact-shadow march saw, as colour (#735).
    ///
    /// Speckle on a floor is one of three things that look identical in
    /// a shaded frame: the surface occluding itself on the march's very
    /// first sample, a real occluder found further along, or a ray too
    /// short in screen space to have marched at all. Red, green and blue
    /// respectively — see `inti_contact_shadow_debug`.
    ///
    /// It shows the **first light that opted in**, because the march is
    /// per light and averaging several would hide the one being looked
    /// at. Magenta means no light in the scene marches.
    ContactShadows = 13,
    /// One light, alone, in greyscale, with its shadow (#743).
    ///
    /// The question is *why is this dark*, and a shaded frame cannot
    /// answer it: no light reaching the surface, a shadow reaching it,
    /// and a dark material all produce the same pixel and have three
    /// different fixes. This removes two of the three — every other
    /// light, and the material's colour — so what is left on screen is
    /// one light's contribution and nothing else.
    ///
    /// Which light is the entity selected in the World panel, resolved
    /// to its slot in the light buffer and carried in `IntiFrame`'s
    /// `debug_light`. Magenta means the selection is not a light this
    /// frame rendered.
    ///
    /// ⚠️ Only a directional light casts a cascade shadow today, so a
    /// point or spot usually shows none. The editor states that next to
    /// the selector — a limitation somebody has to guess at is worse
    /// than one written down.
    SingleLight = 14,
    /// What the spot lights' shadow maps actually contain, and through
    /// which projection they are read (#777).
    ///
    /// Three things look identical in a shaded frame and live in three
    /// different files: the map is empty because the raster drew
    /// nothing; the map has content but was rasterised through a
    /// different matrix than the one being sampled; the map and the
    /// matrix are right and the layer index is wrong.
    ///
    /// - magenta — no spot light in the scene casts
    /// - dark blue — outside this spot's frustum, so nothing to read
    /// - **red** — inside the frustum and the map is EMPTY there: the
    ///   raster pass did not draw
    /// - red/green gradient — the sampled uv, with blue carrying the
    ///   stored depth. Rotate the light: if the gradient does not turn
    ///   with it, the record reaching the shader is not the one the
    ///   pass thinks it wrote
    SpotShadowMap = 15,
}

/// Runtime knob for the cull / LOD selector. Lives as a
/// [`Resource`](kooch_core::resource::Resources) so the editor can
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
            Self::FrustumRejected,
            Self::BackfaceRejected,
            // Falls back to a no-op overlay on the R64 atomic path
            // because `cs_cull_scene_pool_atomic` doesn't run a Hi-Z
            // occlusion test — the cull writes only frustum / backface
            // / lod reasons. The acceptance criteria for #454 explicitly
            // allows this: HiZRejected lights up only when the Hi-Z
            // 2-pass orchestrator (#445 SPD follow-up) runs the scene
            // through `cs_cull_scene_pool_atomic_hi_z`, which still
            // needs its own reject_reasons wiring (separate follow-up).
            Self::HiZRejected,
            Self::Normals,
            Self::ShadowCascades,
            Self::ContactShadows,
            Self::SingleLight,
            Self::SpotShadowMap,
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
            Self::Normals => "Normals",
            Self::ShadowCascades => "Shadow cascades",
            Self::ContactShadows => "Contact shadows",
            Self::SingleLight => "Single light",
            Self::SpotShadowMap => "Spot shadow map",
        }
    }
}

#[cfg(test)]
mod tests;
