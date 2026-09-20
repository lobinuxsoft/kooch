//! Meshlet pipeline debug visualization modes (#451).

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
    /// Force-render ONLY meshlets at LOD 0 (the highest-detail chain entry, `lod_error == 0`).
    /// Bypasses the normal selector so the artist can inspect what the finest-LOD geometry looks
    /// like in isolation, free of any chain-descent overlap.
    OnlyLod0 = 8,
    /// Force-render ONLY meshlets that are roots in the LOD DAG (`parent_meshlet_index ==
    /// MESHLET_ROOT_PARENT`). Shows the coarsest-available representation of each registered mesh
    /// in isolation.
    OnlyRoots = 9,
    /// Bright yellow on meshlets that the frustum test discarded.
    FrustumRejected = 10,
    /// World-space normal painted as colour, modulated by albedo.
    Normals = 11,
    /// What the shadow system sees, as colour (#476).
    ShadowCascades = 12,
    /// What the contact-shadow march saw, as colour (#735).
    ContactShadows = 13,
    /// One light, alone, in greyscale, with its shadow (#743).
    SingleLight = 14,
    /// How many lights each pixel actually evaluates, as a heatmap (#817).
    LightsPerPixel = 15,
    /// A point light's cube map, answering for itself (#852).
    PointShadowFactor = 16,
    /// The cube map itself, all six faces at once (#852).
    PointCubeFaces = 17,
    /// Which mip level each pixel samples, as colour.
    TextureMipLevel = 18,
    /// The HDR frame FSR 3.1 was handed, at the render pixel this output pixel sits in.
    Fsr3Input = 19,
    /// Dilated motion vectors, biased so that zero is grey.
    Fsr3Motion = 20,
    /// Red reactive, green disocclusion, blue how many frames of history the pixel has earned.
    Fsr3Masks = 21,
    /// This frame's upsample alone, with no history blended in.
    Fsr3Upsample = 22,
    /// The reprojected history alone, before rectification.
    Fsr3History = 23,
    /// Red the lock, green the luma instability, blue the upsample's
    /// total weight — the three terms that decide how hard the history
    /// is rectified against the neighbourhood.
    Fsr3Locks = 24,
    /// The two inputs to the upsample weight: red and green the offset from this output pixel to
    /// the render grid, in render pixels, and blue the kernel width (FSR's 1.99 ceiling reads as
    /// full).
    Fsr3Weights = 25,
    /// What the VIRTUAL SHADOW PAGES see, as colour (#866).
    VirtualPages = 26,
    /// The page each pixel MARKED, painted over the scene.
    VirtualPageTiles = 27,
    /// How old the page each pixel reads is, and which clipmap level it came from.
    VirtualPageAge = 28,
    /// Which CUBE FACE of one lamp each pixel reads its shadow page from, and at which chain level.
    LocalPageFaces = 29,
    /// What one lamp's page ANSWERS at each pixel, before the shading mixes it with ninety-nine
    /// others.
    LocalPageDepth = 30,
    /// Every triangle's edges, read off the visibility buffer: a pixel whose neighbour carries
    /// another `(slot, triangle)` is an edge. Shows the polygon load a mesh actually rasterises.
    Wireframe = 31,
    /// The same edges, drawn over the shaded frame: what the mesh is, and where it sits on what it
    /// is drawing.
    WireframeOver = 32,
}

/// Runtime knob for the cull / LOD selector. Lives as a
/// [`Resource`](kooch_core::resource::Resources) so the editor can adjust it in flight without
/// rebuilding the meshlet stage.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct MeshletLodSettings {
    pub target_error_pixels: f32,
    /// Reject an INSTANCE whose bounding sphere covers fewer than this many pixels, before it is
    /// expanded into meshlets (#1002). `0` = off.
    pub min_screen_pixels: f32,
    /// Whether the cull rejects instances first and expands only the survivors (#1002), instead of
    /// dispatching `instances × the heaviest mesh in the scene`.
    pub two_level: bool,
}

impl Default for MeshletLodSettings {
    fn default() -> Self {
        Self {
            target_error_pixels: 1.0,
            // 🔴 Off, because a threshold picked from a whiteboard is how `DEFAULT_PAGES` sat at
            // half of Epic's for a year. The two-level shape is a pure win and ships on; what it
            // hides is a judgement and ships off.
            min_screen_pixels: 0.0,
            two_level: true,
        }
    }
}

impl MeshletDebugMode {
    /// Stable raw discriminant the deferred shader pattern-matches on.
    #[inline]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    /// Modes whose shader implementation is wired and visually validated. The editor's debug-view
    /// dropdown iterates this so users never select a mode that silently falls back to `Off`.
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
            // Falls back to a no-op overlay on the R64 atomic path because
            // `cs_cull_scene_pool_atomic` doesn't run a Hi-Z occlusion test — the cull writes only
            // frustum / backface / lod reasons.
            Self::HiZRejected,
            Self::Normals,
            Self::ShadowCascades,
            Self::ContactShadows,
            Self::SingleLight,
            Self::LightsPerPixel,
            Self::PointShadowFactor,
            Self::PointCubeFaces,
            Self::VirtualPages,
            Self::VirtualPageTiles,
            Self::VirtualPageAge,
            Self::LocalPageFaces,
            Self::LocalPageDepth,
            Self::TextureMipLevel,
            Self::Wireframe,
            Self::WireframeOver,
            // The Fsr3* views are deliberately NOT offered any more: the upscaler's bring-up is
            // done and they earned their retirement from the dropdown (the user's words: "ya los
            // podemos sacar porque andan bien").
        ]
    }

    /// `true` when the mode reads the editor's selected light.
    #[inline]
    pub const fn needs_selected_light(self) -> bool {
        matches!(
            self,
            Self::SingleLight | Self::LocalPageFaces | Self::LocalPageDepth
        )
    }

    /// Reject-reason code the cull shader writes when this mode is active and
    /// `CullParams.debug_active != 0`. Mirrors the `REJECT_REASON_*` constants in
    /// `meshlet_cull/atomic.wgsl`.
    #[inline]
    pub const fn reject_reason_code(self) -> Option<u32> {
        match self {
            Self::FrustumRejected => Some(2),
            Self::BackfaceRejected => Some(3),
            Self::HiZRejected => Some(4),
            _ => None,
        }
    }

    /// `true` when the mode's pipeline writes to an R32Uint atomic storage texture
    /// (triangle-density accumulator, overdraw accumulator, reject-reason buffer).
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

    /// Capability-aware dropdown list. Returns every mode currently wired in
    /// [`Self::all_implemented`], minus those the device cannot run.
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
            Self::LightsPerPixel => "Lights per pixel",
            Self::Wireframe => "Wireframe",
            Self::WireframeOver => "Wireframe over",
            Self::PointShadowFactor => "Point shadow factor",
            Self::PointCubeFaces => "Point cube faces",
            Self::LocalPageFaces => "Lamp shadow pages: faces",
            Self::LocalPageDepth => "Lamp shadow pages: occlusion",
            Self::VirtualPages => "Virtual shadow pages",
            Self::VirtualPageTiles => "Virtual shadow page tiles",
            Self::VirtualPageAge => "Virtual shadow page age",
            Self::TextureMipLevel => "Texture mip level",
            Self::Fsr3Input => "FSR 3.1 — 1 input colour",
            Self::Fsr3Motion => "FSR 3.1 — 2 dilated motion",
            Self::Fsr3Masks => "FSR 3.1 — 3 reactive / disocclusion / accumulation",
            Self::Fsr3Upsample => "FSR 3.1 — 4 upsample, no history",
            Self::Fsr3History => "FSR 3.1 — 5 reprojected history",
            Self::Fsr3Locks => "FSR 3.1 — 6 lock / instability / weight",
            Self::Fsr3Weights => "FSR 3.1 — 7 kernel offset / width",
        }
    }

    /// Which of FSR 3.1's stages this mode asks for, 1-based, or 0 when it is not one of them.
    pub const fn fsr3_stage(self) -> u32 {
        match self {
            Self::Fsr3Input => 1,
            Self::Fsr3Motion => 2,
            Self::Fsr3Masks => 3,
            Self::Fsr3Upsample => 4,
            Self::Fsr3History => 5,
            Self::Fsr3Locks => 6,
            Self::Fsr3Weights => 7,
            _ => 0,
        }
    }

    /// True when Inti resolves this mode inside the shading shader, so
    /// nothing temporal downstream should run.
    pub const fn replaces_shading(self) -> bool {
        // The overlay is the production frame with lines on top: everything that resolves or grades
        // a frame still has to run.
        !self.overlays() && self.as_u32() >= Self::Normals.as_u32() && self.fsr3_stage() == 0
    }

    /// True when the mode draws over a finished frame instead of taking its place.
    #[inline]
    pub const fn overlays(self) -> bool {
        matches!(self, Self::WireframeOver)
    }

    /// True when the fullscreen debug pass draws it off the visibility buffer alone, instead of the
    /// shade resolving it.
    #[inline]
    pub const fn colorizes(self) -> bool {
        matches!(
            self,
            Self::MeshletIds
                | Self::InstanceIds
                | Self::TriangleDensity
                | Self::Overdraw
                | Self::CullPassthrough
                | Self::Wireframe
        )
    }

    /// True when the mode hands back colour that is already ready for the screen, so the tonemap
    /// must pass it through untouched.
    pub const fn is_display_referred(self) -> bool {
        if self.overlays()
            || matches!(
                self,
                Self::Fsr3Input | Self::Fsr3Upsample | Self::Fsr3History
            )
        {
            return false;
        }
        self.as_u32() >= Self::Normals.as_u32()
    }
}

#[cfg(test)]
mod tests;
