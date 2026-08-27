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
    /// How many lights each pixel actually evaluates, as a heatmap
    /// (#817).
    ///
    /// Clustered shading makes cost a property of *where the pixel is*:
    /// a fragment pays for the lights in its froxel and for nothing
    /// else. No pass timing can show that — `raster + shade` is one
    /// number for the whole screen, and it grew from 5.27 ms to 34.92 ms
    /// between a still camera and a moving one without saying which
    /// pixels did it.
    ///
    /// Black is nothing, blue is few, green is the middle of the scale
    /// and red is the top of it — [`kooch_lighting::LightsHot`], which
    /// the editor moves, because the count that separates a busy froxel
    /// from a quiet one in a hundred-light scene washes a four-lamp room
    /// flat red. Directional lights are
    /// included — the grid does not cluster them because they reach
    /// every cell, so leaving them out would under-report what the pixel
    /// pays.
    ///
    /// 🔴 **A frame shading without the grid reads as flat maximum.**
    /// `inti.clustered == 0` means every light is evaluated for every
    /// pixel, which is what the frame cost before #780 and what a path
    /// with no camera matrices still does. That is not a quirk of the
    /// view, it is the view working: a scene that quietly stopped
    /// clustering looks exactly like a scene that is slow for no reason.
    LightsPerPixel = 15,
    /// A point light's cube map, answering for itself (#852).
    ///
    /// "The shadow is not there" is four faults wearing one pixel: no
    /// lamp near this point casts, the point is past the lamp's reach so
    /// there is nothing to block, the cube says lit because the occluder
    /// never reached the map, or the cube says dark and the other lamps
    /// in the room fill it back in. Four different fixes, one colour in
    /// a shaded frame.
    ///
    /// So this paints the cube's answer and nothing else — no BRDF, no
    /// cosine, no exposure, no ambient, no second light. Magenta is no
    /// casting lamp here, blue is past the lamp's `range`, and grey is
    /// the factor itself: black fully occluded, white fully lit.
    ///
    /// Which lamp is whichever is selected in the World panel when that
    /// is a casting point light, and otherwise the first one that casts —
    /// the same lamp for every pixel. Choosing the strongest one per
    /// pixel drew a hard seam where the winner changed, which reads as a
    /// cut shadow: the exact fault the view exists to rule out.
    PointShadowFactor = 16,
    /// The cube map itself, all six faces at once (#852).
    ///
    /// [`Self::PointShadowFactor`] answers "is this point occluded". When
    /// that answer is wrong, two possibilities are left — the map holds
    /// the wrong depth, or it holds nothing because the occluder never
    /// got rasterised into that face — and only opening the texture
    /// separates them.
    ///
    /// The screen becomes a 3x2 grid, one cell per world axis in the
    /// order +X, -X, +Y, -Y, +Z, -Z. Dark blue is nothing recorded,
    /// which is the picture of an occluder culled out of the map; the
    /// grey ramp is distance to the recorded occluder over the lamp's
    /// range. Magenta means no point light casts, or the frame is not
    /// clustered — the screen size is derived from the froxel grid.
    PointCubeFaces = 17,
    /// Which mip level each pixel samples, as colour.
    ///
    /// 🔴 Built because a screenshot cannot tell three faults apart: a
    /// mip chain that was never generated, a chain that was generated
    /// wrong, and a chain that is fine while the LOD selection asks for
    /// the wrong level. All three look like the same flat grey surface,
    /// and the third one is invisible to every test in the suite —
    /// found by the owner, who noticed the texture looked identical
    /// however close the camera got.
    ///
    /// The level is computed the way the hardware computes it: the
    /// footprint of the uv derivatives in texels, `log2` of the longer
    /// axis. Each whole level gets its own colour, so a correct frame
    /// shows **bands** that move when the camera does — near the camera
    /// the low levels, toward the horizon the high ones. A frame stuck
    /// on one colour is a LOD that is not listening, which is the whole
    /// question.
    ///
    /// Magenta means the material has no albedo map, so there is no
    /// chain to select from and nothing to say.
    TextureMipLevel = 18,
    /// The HDR frame FSR 3.1 was handed, at the render pixel this output
    /// pixel sits in.
    ///
    /// 🎯 The first step of a staircase, and the six of them are unlike
    /// every mode above: they do NOT replace the shading. They leave the
    /// upscaler running and make it write one of its own intermediates
    /// instead of the image, in the order the data flows through it. The
    /// first step that looks wrong is the first pass that IS wrong, and
    /// everything after is downstream of the same fault.
    ///
    /// Built because six dispatches write five intermediates no eye ever
    /// sees, so a wrong frame says nothing about which one produced it.
    ///
    /// ⚠️ They corrupt the history while they are on — the point is to
    /// see one stage, not to keep a valid frame — and with any other
    /// technique selected they show the ordinary image, because there is
    /// no FSR running to ask.
    Fsr3Input = 19,
    /// Dilated motion vectors, biased so that zero is grey.
    ///
    /// ⚠️ Uniform grey with a still camera is CORRECT. A still camera
    /// cannot test the motion path at all, which is exactly how a
    /// reversed reprojection survived two tests.
    Fsr3Motion = 20,
    /// Red reactive, green disocclusion, blue how many frames of history
    /// the pixel has earned.
    ///
    /// Blue should climb to full over three still frames. Staying black
    /// is the accumulation counter never advancing, which starves every
    /// term downstream.
    Fsr3Masks = 21,
    /// This frame's upsample alone, with no history blended in.
    ///
    /// 🎯 The step that cuts the technique in half: if the scene appears
    /// here, the inputs and the Lanczos kernel are fine and the fault is
    /// in the history path.
    Fsr3Upsample = 22,
    /// The reprojected history alone, before rectification.
    Fsr3History = 23,
    /// Red the lock, green the luma instability, blue the upsample's
    /// total weight — the three terms that decide how hard the history
    /// is rectified against the neighbourhood.
    Fsr3Locks = 24,
    /// The two inputs to the upsample weight: red and green the offset
    /// from this output pixel to the render grid, in render pixels, and
    /// blue the kernel width (FSR's 1.99 ceiling reads as full).
    ///
    /// 🎯 Measured after the sum itself came back zero everywhere with
    /// both of these apparently in range. Either red or green above
    /// 1.0 means no tap of the 3x3 can land in the kernel's positive
    /// lobe, and the accumulation can never take a new sample.
    Fsr3Weights = 25,
    /// What the VIRTUAL SHADOW PAGES see, as colour (#866).
    ///
    /// The sibling of [`Self::ShadowCascades`], for the other technique.
    /// A missing paged shadow is one of three things that look identical
    /// in a shaded frame, and they have three different fixes:
    ///
    /// - the reader finds **no page** at any clipmap level, so marking
    ///   and sampling disagree about which page covers this point;
    /// - it finds a page that was **allocated and never drawn into**, so
    ///   the cull or the expansion dropped the caster for that page;
    /// - it finds a page with real depth and the **comparison** says
    ///   lit, so the bias or the depth space is wrong.
    ///
    /// Red, yellow and green respectively; blue is a point the pages do
    /// shadow, and magenta means the paged path is not running at all.
    /// Brightness is the clipmap level the answer came from, so the
    /// bands are visible without losing the classification.
    ///
    /// 🎯 Built after guessing twice — starvation, then winding — and
    /// being right only once. Three causes that look alike need a view
    /// that separates them, not a third hypothesis.
    VirtualPages = 26,
    /// The page each pixel MARKED, painted over the scene.
    ///
    /// Hue is the clipmap level — where the frame is spending detail —
    /// and brightness is the page identity, hashed, so the tiling is
    /// visible. A page covering a quarter of the screen is a page too
    /// coarse for it; a mosaic too fine to resolve is detail nobody
    /// sees.
    ///
    /// 🔴 The other half of [`Self::VirtualPages`], and it lives HERE
    /// rather than in `RenderSettings` because a debug view that is a
    /// checkbox in the settings panel is a debug view nobody finds. Two
    /// halves of one question belong in one list: this one is what the
    /// marking pass CHOSE, that one is what the reader FOUND.
    VirtualPageTiles = 27,
    /// How old the page each pixel reads is, and which clipmap level it
    /// came from.
    ///
    /// 🔴 Built to make a FLICKER readable. A shadow that blinks while
    /// the camera moves is four faults wearing one coat, and
    /// [`Self::VirtualPages`] cannot separate them because it answers a
    /// still frame. This answers what CHANGED, on three independent
    /// signals:
    ///
    /// - **White** — the page has NO CONTENT: resident, correctly
    ///   addressed, and never drawn into. Whatever its slot holds
    ///   belonged to whoever had the slot last, so the depth test
    ///   answers against a stranger's geometry.
    ///
    ///   🔴 This used to read "allocated this frame" and painted the
    ///   whole screen white, always. Word 1 is the frame a page was
    ///   last REQUESTED and the marking asks for every visible page
    ///   every frame, so `frame - age` was zero everywhere and the view
    ///   returned before reaching its own hue and fade. It could not
    ///   answer the question it was built for, which is how it went
    ///   unnoticed: a view that is always white looks like a view of
    ///   something.
    /// - **Hue** — the clipmap level the walk stopped at. A band that
    ///   jumps between two colours frame to frame is the reader crossing
    ///   a level boundary, which moves the texel size and the rect under
    ///   it.
    /// - **Brightness** — frames since the page was last requested,
    ///   full at one and dim by sixteen.
    ///
    ///   ⚠️ INERT for anything on screen, for the reason above: the
    ///   marking refreshes that word every frame it asks. It needs the
    ///   frame a page was ALLOCATED, recorded apart from the frame it
    ///   was requested, and that word does not exist yet.
    ///
    /// Black is no page at any level; magenta is the paged path off.
    VirtualPageAge = 28,
    /// Which CUBE FACE of one lamp each pixel reads its shadow page
    /// from, and at which chain level.
    ///
    /// 🔴 The sun has one direction and a lamp has SIX, so a lamp's page
    /// arithmetic carries six sign choices the sun's does not — and
    /// every one of them is invisible in the shaded image. A shadow on
    /// the wrong wall, a shadow that vanishes when the camera crosses an
    /// axis, a shadow that mirrors: all of them look like a bad matrix
    /// and all of them are a face.
    ///
    /// The lamp is fixed by `debug_light`, on purpose. Painting a
    /// hundred lamps at once averages exactly the signal being looked
    /// for.
    ///
    /// - **Six hues** — the face. A sphere around the lamp should read
    ///   as six clean patches with straight seams. Torn seams, mirrored
    ///   patches or a face appearing twice is a sign error in
    ///   `face_dir` / `cube_face`.
    /// - **Brightness** — the chain level the walk stopped at.
    /// - **White** — the lamp reaches this pixel and there is NO page.
    ///   That separates a marking fault from a residency one: white
    ///   where the lamp clearly lights means the page was never
    ///   allocated, not that the face is wrong.
    /// - **Black** — outside the lamp's range. **Magenta** — paged path
    ///   off.
    LocalPageFaces = 29,
    /// What one lamp's page ANSWERS at each pixel, before the shading
    /// mixes it with ninety-nine others.
    ///
    /// 🔴 The other half of [`Self::LocalPageFaces`], and the split is
    /// the same one that made the sun's flicker readable: that view says
    /// which page was found, this one says what it contained. A shadow
    /// that looks wrong is either reading the wrong page or reading the
    /// right page and comparing wrong, and no single view can say which.
    ///
    /// - **Red** — the page says occluded. **Green** — lit.
    /// - **Blue** — the lamp reaches here and no page does.
    /// - **Black** — out of range. **Magenta** — paged path off.
    ///
    /// A lamp whose faces are clean here and whose shadow is still wrong
    /// in the frame is a lamp the SHADING is mixing wrong, which is a
    /// different pass entirely.
    LocalPageDepth = 30,
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
#[derive(Copy, Clone, Debug, PartialEq)]
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
            Self::LightsPerPixel,
            Self::PointShadowFactor,
            Self::PointCubeFaces,
            Self::VirtualPages,
            Self::VirtualPageTiles,
            Self::VirtualPageAge,
            Self::LocalPageFaces,
            Self::LocalPageDepth,
            Self::TextureMipLevel,
            // The Fsr3* views are deliberately NOT offered any more:
            // the upscaler's bring-up is done and they earned their
            // retirement from the dropdown (the user's words: "ya los
            // podemos sacar porque andan bien"). The variants stay —
            // the taps and the tests behind them are how the next
            // upscaler regression gets diagnosed — but out of the way.
        ]
    }

    /// `true` when the mode reads the editor's selected light.
    ///
    /// A predicate rather than an equality at the call site: a view
    /// that reads `IntiFrame::debug_light` and is not listed here gets
    /// `None` and renders its "nothing selected" branch forever, with
    /// nothing to suggest the fault is in another crate. That already
    /// happened once, to a view since removed — and then again to both
    /// lamp page views, which shipped painting the screen magenta.
    ///
    /// `every_view_that_reads_the_selected_light_is_listed` is what
    /// stops the third time: it scans the shader rather than trusting
    /// this list to be maintained.
    #[inline]
    pub const fn needs_selected_light(self) -> bool {
        matches!(
            self,
            Self::SingleLight | Self::LocalPageFaces | Self::LocalPageDepth
        )
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
            Self::LightsPerPixel => "Lights per pixel",
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

    /// Which of FSR 3.1's stages this mode asks for, 1-based, or 0 when
    /// it is not one of them.
    ///
    /// 🔴 These are the only debug modes that do not replace the
    /// shading, so [`Self::replaces_shading`] must exclude them or the
    /// upscaler they are meant to inspect never runs.
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
        self.as_u32() >= Self::Normals.as_u32() && self.fsr3_stage() == 0
    }

    /// True when the mode hands back colour that is already ready for
    /// the screen, so the tonemap must pass it through untouched.
    ///
    /// 🔴 Three of FSR's steps are the exception, and the reason is a
    /// mistake this made the first time: they show RADIANCE — the input
    /// frame, the upsample, the history — and radiance without the
    /// filmic curve is nearly black in any dimly-lit scene. A debug view
    /// that reads black when the data is fine is worse than no view at
    /// all, because it sends the search after a defect that is not
    /// there. So those three go through the ordinary tonemap and are
    /// directly comparable with the real image; the steps that show a
    /// 0..1 quantity bypass it and read as a grey ramp.
    pub const fn is_display_referred(self) -> bool {
        if matches!(
            self,
            Self::Fsr3Input | Self::Fsr3Upsample | Self::Fsr3History
        ) {
            return false;
        }
        self.as_u32() >= Self::Normals.as_u32()
    }
}

#[cfg(test)]
mod tests;
