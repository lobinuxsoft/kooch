//! `RenderSettings` — what the **author** decided the project looks like
//! (#744).
//!
//! Exposure and ambient light are decisions someone makes once for a
//! project and keeps. Until this, they were `Resources` with defaults
//! and no way to change them: #441 built the control and left it out of
//! reach, which is the engine's recurring failure committed knowingly.
//!
//! # Author settings, not player settings
//!
//! This ships with the game and belongs in version control. What the
//! **player** picks — resolution, volume, key bindings — is #736, lives
//! under `~/.config/` and is not committed. Every engine keeps the two
//! apart, and merged they would put an artist's exposure and a player's
//! volume slider in the same file when exactly one of them belongs in a
//! commit.
//!
//! # Why an asset
//!
//! Because the machinery exists. A RON loader that registers itself,
//! reflection so the Inspector edits it, the save-and-refresh path from
//! #728, and the asset browser as its home. The alternative is a bespoke
//! settings panel, and the evidence that bespoke panels do not get built
//! is that this setting had none for as long as it existed.
//!
//! # Why the fields are flat
//!
//! `PhysicalCamera` and `AmbientLight` are structs, and nesting them
//! would need `FieldKind::Nested`. Flat fields with doc comments give
//! the generic editor something to draw and give each value a tooltip
//! that states its unit — which is the entire reason someone opens this
//! asset.

use kooch_core::asset_loader::{AssetError, AssetLoader, AssetResult, LoadContext};
use kooch_core::resource::Resources;
use kooch_ecs::Reflect;
use kooch_lighting::{AmbientLight, Exposure, PhysicalCamera};
use serde::{Deserialize, Serialize};

use crate::contact_shadow::ContactShadowSettings;
use crate::shadow::ShadowSettings;

/// Extension a settings file carries.
pub const RENDER_SETTINGS_EXTENSION: &str = "rendersettings";

/// How a project looks, as the author set it.
///
/// Absent, the defaults below apply and the project renders exactly as
/// it would with no file at all. Missing configuration is not an error:
/// a new project must not need a file it never asked for.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(category = "Rendering")]
pub struct RenderSettings {
    /// Aperture, as an f-stop. **Lower is brighter**: f/1.4 gathers four
    /// times the light of f/2.8.
    ///
    /// Exposure is expressed as a camera because EV100 is a correct
    /// number and an unusable control. f/16 in bright sun, f/1.0 indoors.
    #[serde(default = "default_aperture")]
    #[reflect(group = "Exposure")]
    pub aperture_f_stops: f32,
    /// Shutter time in SECONDS. Longer is brighter. 1/125 is 0.008.
    #[serde(default = "default_shutter")]
    #[reflect(group = "Exposure")]
    pub shutter_speed_s: f32,
    /// Film speed. Higher is brighter — and in a real camera noisier,
    /// though here it is brightness only.
    #[serde(default = "default_iso")]
    #[reflect(group = "Exposure")]
    pub sensitivity_iso: f32,

    /// Ambient light arriving from world up, as linear RGB.
    ///
    /// Stands in for a sky the renderer cannot sample yet. Without it a
    /// metal facing away from every light renders pure black — correct
    /// for the model, and indistinguishable from a bug.
    #[serde(default = "default_sky")]
    #[reflect(group = "Ambient light")]
    pub ambient_sky_color: glam::Vec3,
    /// Ambient light arriving from world down, as linear RGB. Bounce
    /// off the ground, not sky.
    #[serde(default = "default_ground")]
    #[reflect(group = "Ambient light")]
    pub ambient_ground_color: glam::Vec3,
    /// Ambient illuminance in LUX, on the same scale as a directional
    /// light. An office is 320; a directional light defaults to 10 000.
    ///
    /// Raise it and shadowed surfaces lift; raise it far and the scene
    /// flattens, because ambient arrives from everywhere and therefore
    /// describes no direction.
    #[serde(default = "default_ambient_intensity")]
    #[reflect(group = "Ambient light")]
    pub ambient_intensity: f32,

    /// Whether the sun's shadow comes from the VIRTUAL page pool
    /// instead of the four cascades (#866).
    ///
    /// 🔴 It REPLACES the cascades rather than blending with them. Two
    /// techniques over one surface disagree at their own boundaries, and
    /// the disagreement reads as a seam that belongs to neither.
    ///
    /// A page is allocated where the frame actually needs one, so detail
    /// follows the screen instead of following four fixed distances —
    /// and the budget becomes memory rather than a count of slots.
    ///
    /// ⚠️ The pages sampled this frame were marked and rasterised the
    /// PREVIOUS one. The raster and the shading are a single fused
    /// fragment shader here, so there is no depth buffer to mark from
    /// until shading is over. A page that appears suddenly — a fast
    /// camera turn, an object entering frame — is lit for one frame
    /// rather than wrong, which is the failure mode to have.
    ///
    /// ⚠️ Local lights still use their cube maps: their pages are
    /// marked and allocated, and rasterising them needs a cull per
    /// view, which is 4848 views against the sun's 17.
    #[serde(default = "default_virtual_shadows")]
    #[reflect(group = "Shadows: virtual pages")]
    pub virtual_shadows: bool,
    /// Shadow texels per screen pixel, as a PERCENTAGE.
    ///
    /// 🔴 **The lever the page census found, and the only one that
    /// moved.** A virtual shadow map allocates pages to match the
    /// screen's detail, so this — not the page size, not the virtual
    /// size — is what decides the bill: page count falls roughly with
    /// its square, because a coarser texel is a level coarser in both
    /// axes.
    ///
    /// 100 is Epic's ask, one shadow texel per screen pixel, and it is
    /// what every figure measured for #866 was taken at. Measured on
    /// `many_lights.scene` with **all 101 lights casting**, a 400x400
    /// view already wanted 2581 pages — 161 MiB, past this engine's
    /// 152 MiB of fixed allocations, at a resolution smaller than a
    /// thumbnail.
    ///
    /// 🔴 100 is the CEILING. It is a quality option and the top of one
    /// is the top; the values above it existed to reach the cascade's
    /// resolution and their absence is the honest statement that this
    /// path does not, yet. See `default_shadow_density`.
    ///
    /// ⚠️ Below 100 a shadow's edge is softer than the surface it falls
    /// on, which reads as blur rather than as a lower setting. 50 is a
    /// quarter of the pages and the point where it starts to show.
    #[serde(default = "default_shadow_density")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_DENSITY_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_density: u32,
    /// Physical pages the pool holds, which IS the memory budget: one
    /// page is 64 KiB at 128 texels and `Depth32Float`.
    ///
    /// 🔴 Read it against what stands today — **152 MiB of fixed shadow
    /// allocations for four casting lights**, whether or not they cast.
    /// 2048 pages is 128 MiB and adapts to the frame. Epic's own default
    /// is 4096 for a whole scene, 6144 for open worlds, and 8192 is
    /// where their notes say it thrashes.
    ///
    /// ⚠️ Overflow is not graceful anywhere: pages the pool cannot seat
    /// render unshadowed, and Epic's shows up as checkerboard
    /// corruption. The performance panel names it rather than leaving it
    /// to be recognised by sight.
    #[serde(default = "default_shadow_pool_pages")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_POOL_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_pool_pages: u32,
    /// Width of the page shadows' PCF footprint, in shadow texels.
    ///
    /// 1 is the comparison-bilinear the cube path gets from hardware —
    /// smooth edge, no softness. Wider widths box-filter over the
    /// footprint with bilinear sub-texel weights, Castano-style: the
    /// penumbra grows and the cost is the taps, `(width + 1)²` loads
    /// per light per pixel. The taps still clamp to the page — a page's
    /// neighbour texel can belong to another level or another light,
    /// which is why no hardware sampler can do this (#941).
    #[serde(default = "default_shadow_softness")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_SOFTNESS_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_softness: u32,
    /// Projected radius, in screen pixels, under which a local light
    /// casts no shadow pages at all (#944). The light still SHADES —
    /// only its shadow is judged not worth the pool it would spend:
    /// a lamp whose whole reach covers forty pixels asks for the same
    /// six-face mip chain as one filling the screen. Epic runs the
    /// same gate as a pass, `PruneLightGridCS`, before anything marks.
    ///
    /// 0 disables the gate. The sun is never gated — it has no radius.
    #[serde(default = "default_shadow_min_pixels")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_MIN_PIXELS_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_min_pixels: u32,
    /// How far a shadow lookup steps along the receiver's NORMAL before
    /// comparing, as a MULTIPLE OF THE CLIPMAP TEXEL it landed on.
    ///
    /// 🔴 It multiplies a texel, and a clipmap texel is 0.1 mm at level
    /// 0 and 5.12 m at level 16. So this is not a distance: it decides
    /// one, and the one it decides spans five orders of magnitude
    /// across the chain. Raise it to kill acne, and the far levels lose
    /// their shadows first.
    #[serde(default = "default_shadow_normal_bias")]
    #[reflect(
        group = "Shadows: virtual pages",
        range = SHADOW_NORMAL_BIAS_RANGE,
        shown_when = PAGES_ON
    )]
    pub shadow_normal_bias: f32,
    /// How far the same lookup steps TOWARDS the light, in metres.
    /// Constant across the chain, unlike the normal step above.
    #[serde(default = "default_shadow_depth_bias")]
    #[reflect(
        group = "Shadows: virtual pages",
        range = SHADOW_DEPTH_BIAS_RANGE,
        shown_when = PAGES_ON
    )]
    pub shadow_depth_bias: f32,
    /// A ceiling on the world-space normal step, in METRES. 0 disables
    /// it, which is what shipped.
    ///
    /// 🔴 Without a cap the step follows the texel: 0.58 m at clipmap
    /// level 12, 2.3 m at 14, 9.2 m at 16. A receiver pushed metres
    /// along its own normal leaves the volume its caster shadows, so
    /// the depth test answers LIT — and the shadow ends in a straight
    /// line at the level boundary, with the page present, resident and
    /// correctly drawn. That failure is indistinguishable from a
    /// missing page in a shaded frame; `Virtual shadow pages` paints it
    /// GREEN, which is what separates the three.
    ///
    /// ⚠️ Too low and the acne the normal step exists to prevent comes
    /// back on the fine levels. This is a ceiling, not a replacement.
    #[serde(default = "default_shadow_bias_max")]
    #[reflect(
        group = "Shadows: virtual pages",
        range = SHADOW_BIAS_MAX_RANGE,
        shown_when = PAGES_ON
    )]
    pub shadow_bias_max: f32,
    /// A ceiling on the receiver's own depth GRADIENT, as a slope —
    /// `tan` of the incidence, per axis.
    ///
    /// 🔴 The reader gives every filter tap the depth the receiving
    /// PLANE has where that tap looks, rather than the depth under the
    /// pixel. That is what a tilted receiver needs and what no scalar
    /// bias can supply: how much depth a tap crosses depends on WHICH
    /// WAY it moved, so one number has to cover the worst axis on every
    /// axis and detaches the shadow along the one that needed nothing.
    ///
    /// ⚠️ A ceiling because the slope diverges as the surface turns
    /// edge-on, and an unbounded one extrapolates a tap to any depth at
    /// all — a lit pixel in the middle of a shadow. 4 is `tan 76°`;
    /// 0 disables the term and restores one depth for every tap.
    #[serde(default = "default_shadow_bias_slope")]
    #[reflect(
        group = "Shadows: virtual pages",
        range = SHADOW_BIAS_SLOPE_RANGE,
        shown_when = PAGES_ON
    )]
    pub shadow_bias_slope: f32,
    /// March the shadow atlas instead of sampling one texel through a
    /// PCF box.
    ///
    /// 🔴 The two readers ask different questions. The box asks what is
    /// stored under this pixel and filters the answer, so an occluder
    /// that did not land in that texel is not found and the pixel comes
    /// out LIT — a hole inside a shadow with the page present, resident
    /// and correctly drawn. Widening the box does not repair it: every
    /// tap is still one comparison at one place, and the taps that miss
    /// vote lit.
    ///
    /// The march asks whether anything blocks along a ray, several
    /// samples per ray, over rays spread across the sun's angular size
    /// (`sun_softness`). It carries no bias constant at all — each step
    /// compares against how far the ray's own depth moved since the
    /// last, which tightens on a surface facing the sun and loosens on
    /// a grazing one by exactly the geometry's own amount. Unreal's
    /// SMRT is the same shape.
    ///
    /// ⚠️ Costs rays times steps of page lookups where the box costs
    /// `(width + 1)²` taps in one page. Measure before shipping it on.
    #[serde(default)]
    #[reflect(group = "Shadows: virtual pages", shown_when = PAGES_ON)]
    pub shadow_page_march: bool,
    /// Runs the expansion from the GEOMETRY: one thread per surviving
    /// meshlet, descending the page pyramid to the pages it lands in,
    /// instead of pairing every listed page against every survivor
    /// (#1022). The sun's clipmap only — a lamp's pages are six
    /// frustums and a different grid.
    ///
    /// Unreal's arrangement. It makes one decision where the pass makes
    /// two: the marking commits a page because a RECEIVER asked for it
    /// and the cull produces survivors from the light's own view, and
    /// nothing checks that the two agree.
    ///
    /// ⚠️ The pairs it emits are the same pairs — the tests that decide
    /// which survive are one shared function. This is a COST switch,
    /// and a picture that changes with it is a finding, not a feature.
    #[serde(default)]
    #[reflect(group = "Shadows: virtual pages", shown_when = PAGES_ON)]
    pub shadow_page_geometry: bool,
    /// Whether Olsson's receiver bound may reject a caster (#940, #949).
    ///
    /// 🔴 A falsifier, not a quality knob. This is the only thing left
    /// in the expansion that DELETES geometry: a caster whose nearest
    /// point lies beyond a page's furthest RECORDED receiver is dropped.
    /// The record comes from the marking, so a page whose furthest
    /// receiver was never marked drops a caster that really does shadow
    /// it — and the page then renders lit with every counter healthy.
    ///
    /// Turn it off and the picture cannot get worse in the other
    /// direction: the worst case is casters paired that occlude
    /// nothing. If a missing shadow comes back with this off, the bound
    /// is where it went.
    #[serde(default = "default_true")]
    #[reflect(group = "Shadows: virtual pages", shown_when = PAGES_ON)]
    pub shadow_page_receiver_bound: bool,
    /// How far, in PAGES, a receiver dilates its page request — Epic's
    /// `PageDilationOffset`. 0 turns it off.
    ///
    /// 🔴 What it buys is a FRAME, not resolution. `vbuf64.render`
    /// rasterises and shades in one pass, so the atlas the shading
    /// samples is a frame old: a page allocated this frame is read with
    /// whatever its slot held last frame — cleared, which is far depth
    /// under reversed-Z, which every reader answers "nothing occludes
    /// here". A lit hole for one frame, every time a page turns over,
    /// and standing on a clipmap level boundary turns them over
    /// continuously.
    ///
    /// A halo asks for the page before the camera reaches it, so the
    /// content is already there when something samples it. It costs
    /// residency: a receiver requests up to three pages instead of one,
    /// and neighbours collapse onto the same page often enough that the
    /// real figure is well under 3x — read `resident` on the panel.
    #[serde(default = "default_shadow_page_halo")]
    #[reflect(group = "Shadows: virtual pages", shown_when = PAGES_ON)]
    pub shadow_page_halo: f32,
    /// How much simplification error a meshlet may show before the cull
    /// picks a finer level, in PIXELS.
    ///
    /// This is the quality-against-cost lever of a meshlet renderer.
    /// The cull keeps the finest level whose own error fits under this
    /// and whose parent's does not, so raising it walks every object
    /// down its LOD chain at once: at 2.0 px a mesh is drawn with
    /// roughly half the triangles it needs at 1.0, and on a handheld
    /// rendering at 480x270 that difference is not visible.
    ///
    /// 🔴 A continuous slider rather than a set of choices, because
    /// this is a quantity to be TUNED against a picture and a frame
    /// time, not one of five named states. Five labelled steps would be
    /// a decision about where the interesting values are, made by
    /// whoever wrote the list rather than by whoever is measuring.
    ///
    /// ⚠️ The floor is 0.01 and not 0. Zero means no level is ever fine
    /// enough, the cull emits nothing, and the screen goes black with a
    /// plausible-looking number behind it.
    ///
    /// ⚠️ It reaches the CAMERA's cull only. The virtual page raster
    /// deliberately holds its own target at one texel: a clipmap level
    /// is already a texel density and the cull is handed that density
    /// directly, so applying the screen's target on top would be the
    /// double relaxation this project already removed from the
    /// cascades.
    #[serde(default = "default_meshlet_lod_error")]
    #[reflect(group = "Geometry", range = MESHLET_LOD_ERROR_RANGE)]
    pub meshlet_lod_error: f32,
    /// Projected radius, in pixels, under which an instance is dropped
    /// before it becomes meshlets (#1002). `0` = draw everything the
    /// frustum holds, which is what shipped.
    ///
    /// 🔴 This is the render distance, and it is a SIZE rather than a
    /// distance on purpose: the projection is infinite reverse-Z and
    /// `far` never arrives. A metre threshold would have to be
    /// re-authored per scene; eight pixels means the same thing in
    /// every one.
    #[serde(default = "default_meshlet_min_pixels")]
    #[reflect(group = "Geometry", range = MESHLET_MIN_PIXELS_RANGE)]
    pub meshlet_min_pixels: f32,
    /// Reject instances first and expand only the survivors (#1002),
    /// instead of dispatching `instances × the heaviest mesh`.
    #[serde(default = "default_meshlet_two_level")]
    #[reflect(group = "Geometry")]
    pub meshlet_two_level: bool,
    /// How far a local light may cast shadow pages from, in multiples
    /// of its OWN range. The light still SHADES past it; only its
    /// shadow stops being paid for.
    ///
    /// 🔴 A second gate beside [`Self::shadow_min_pixels`], and not a
    /// duplicate of it. That one is a projected SIZE, so the distance it
    /// implies scales with the light's range and with the viewport: at
    /// 808x439 a range-50 light does not fall under eight pixels until
    /// 2.4 km, and the threshold that would cut it at a hundred metres
    /// also cuts a small light standing beside the camera. One number
    /// cannot answer both questions. This one scales with the light.
    ///
    /// 0 disables it, which is what shipped. The sun is never gated —
    /// it has no position to be far from.
    #[serde(default = "default_shadow_light_reach")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_LIGHT_REACH_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_light_reach: u32,
    /// Whether shadows are drawn at all. Off frees the atlas entirely
    /// — 64 MiB at the default resolution — and the cube maps with it.
    ///
    /// 🔴 It governs BOTH techniques, not just the cascades its group
    /// used to sit in: `inti_shadow` returns fully lit on this flag
    /// before it ever reaches the branch that picks pages over
    /// cascades, and the whole shadow pass returns early on it.
    #[serde(default = "default_shadows_enabled")]
    #[reflect(group = "Shadows")]
    pub shadows_enabled: bool,
    /// How far from the camera shadows are drawn, in METRES.
    ///
    /// Raising this does not add shadows in the distance so much as move
    /// texels there: the four cascades are fitted to whatever range they
    /// are given, so a larger distance blurs the shadows near the
    /// camera, which are the ones being looked at.
    ///
    /// ⚠️ Cascades ONLY. It reaches `build_cascades` and nothing else —
    /// spot and point maps are fitted from their own light's range, and
    /// the page clipmap carries its own reach.
    #[serde(default = "default_shadow_distance")]
    #[reflect(group = "Shadows: sun cascades", shown_when = PAGES_OFF)]
    pub shadow_distance: f32,
    /// How soft shadow edges get with distance: the TANGENT of the sun's
    /// angular radius, so 0.03 widens a shadow by three centimetres per
    /// metre of gap between the object and what its shadow lands on.
    ///
    /// The honest value for our sun is 0.005, and at that width a soft
    /// shadow is indistinguishable from a hard one. Raise it for an
    /// overcast look; drop it to zero for a hard edge.
    ///
    /// ⚠️ Cascades ONLY. The page reader samples a raw 2x2 and has no
    /// blocker search to widen, so a softness set here does nothing
    /// while the virtual pages are on — which is the whole reason the
    /// page path is sharper-edged than the cascade path at the same
    /// resolution.
    #[serde(default = "default_sun_softness")]
    #[reflect(group = "Shadows: sun cascades", shown_when = PAGES_OFF)]
    pub sun_softness: f32,
    /// Where the first shadow cascade ends, in METRES. The other three
    /// follow logarithmically out to `shadow_distance`.
    ///
    /// **This is the one number that decides shadow sharpness near the
    /// camera.** Lower it and the near cascade covers less ground with
    /// the same texels; raise it and everything close gets coarser.
    /// Unity ships 10.05 and Godot 10.
    ///
    /// ⚠️ Cascades ONLY: there are no splits to place when the sun's
    /// shadow comes from the page clipmap.
    #[serde(default = "default_first_cascade")]
    #[reflect(group = "Shadows: sun cascades", shown_when = PAGES_OFF)]
    pub shadow_first_cascade_distance: f32,

    /// Side of one atlas layer in TEXELS. The atlas is twice this on
    /// each axis: 2048 costs 64 MiB, 1024 costs 16.
    ///
    /// 🔴 NOT cascade-only, whatever the name says. `ShadowAtlas` is one
    /// texture array and this is the side of every layer in it — the
    /// four cascades AND the layer each casting spot light draws into.
    ///
    /// ⚠️ It stops acting entirely once the virtual pages are on, and
    /// this comment used to claim the opposite. `classic_shadow_alloc`
    /// returns a token 256 in that mode and never reads this field, so
    /// the number in the panel described an atlas nobody allocated.
    /// Hidden rather than removed: the classic path is still the whole
    /// of what runs with the pages off.
    #[serde(default = "default_cascade_texels")]
    #[reflect(group = "Shadows: atlas", shown_when = PAGES_OFF)]
    pub shadow_cascade_texels: u32,

    /// How many point lights may cast a cube map at once (#849).
    ///
    /// 🔴 The number that decides whether shadows **pop**. The cubes go
    /// to the lights that matter most from where the camera is, so with
    /// more casting lights on screen than cubes, moving reassigns them
    /// and a shadow appears or disappears for no authored reason.
    ///
    /// **6 MiB of VRAM each.** 4 is 24 MiB, 32 is 192 — on a handheld
    /// that memory is the system's, so this is a real trade and not a
    /// quality slider.
    ///
    /// ⚠️ Everything above holds with the pages OFF, and this comment
    /// used to say the pages did not affect it — true when the page
    /// raster was the sun's only, and false since the local-light raster
    /// landed. The lamps cast from pages now; `classic_shadow_alloc`
    /// drops the cube array to a single 16-texel face and never reads
    /// this field. Neither the VRAM it describes nor the popping it
    /// warns about exists in that mode.
    #[serde(default = "default_point_shadows")]
    #[reflect(group = "Shadows: atlas", shown_when = PAGES_OFF)]
    pub point_shadows: u32,

    /// Steps a contact-shadow ray takes. **Zero turns contact shadows
    /// off** for the whole project, whatever the individual lights say.
    ///
    /// Contact shadows are the few centimetres the cascades cannot
    /// resolve — where an object meets the floor. Cost is per light that
    /// opted in, per pixel it touches.
    #[serde(default = "default_contact_steps")]
    #[reflect(group = "Shadows: contact")]
    pub contact_shadow_steps: u32,
    /// How far a contact-shadow ray travels, in METRES. Longer grounds
    /// objects that hover further from what they stand on, and costs the
    /// same — the steps just spread wider.
    #[serde(default = "default_contact_length")]
    #[reflect(group = "Shadows: contact")]
    pub contact_shadow_length: f32,
    /// Thickness the march assumes every surface has, in METRES.
    ///
    /// The depth buffer records a surface, not a solid, so the march has
    /// to be told how deep to treat one. Too small and contact shadows
    /// detach from thin geometry; too large and a railing shadows
    /// everything behind it.
    #[serde(default = "default_contact_thickness")]
    #[reflect(group = "Shadows: contact")]
    pub contact_shadow_thickness: f32,
    /// March once per pixel — for the light that lit it hardest —
    /// instead of once for every light that reaches it (#845).
    ///
    /// 🔴 On for good reason: the march is linear in taps and has no cap
    /// otherwise. Measured on the OneXFly it costs 1.7 ms per step, and
    /// ~14 lights reach a pixel in a lit scene, which is the whole 13.9
    /// ms frame budget spent on contact alone.
    ///
    /// Turn it off for a scene lit by two or three lights, where each
    /// contact is visible. Under a dozen, the second-brightest lamp's
    /// contact is diluted past seeing anyway.
    #[serde(default = "default_contact_dominant")]
    #[reflect(group = "Shadows: contact")]
    pub contact_shadow_dominant: bool,

    /// Shading as a COMPUTE pass over the visibility buffer (#824)
    /// rather than a fragment one.
    ///
    /// The compute path keeps each tile's froxel lights in workgroup
    /// memory, so the lights are read once per tile instead of once per
    /// pixel. It is also the only path that can shade at a reduced rate
    /// or accumulate frames — half rate and temporal anti-aliasing both
    /// do nothing without it.
    #[serde(default = "default_compute_shading")]
    #[reflect(group = "Shading")]
    pub compute_shading: bool,
    /// Pixels per shaded sample, per AXIS (#825). 1 shades every pixel;
    /// 2 shades one per 2x2 quad and reconstructs the rest using the
    /// visibility buffer as the edge guide.
    ///
    /// Geometry, depth and the visibility buffer stay at full
    /// resolution on every setting — only the lighting evaluation moves.
    #[serde(default = "default_shading_rate")]
    #[reflect(group = "Shading", choices = SHADING_RATE_CHOICES)]
    pub shading_rate: u32,

    /// Temporal anti-aliasing (#481): each frame samples a different
    /// sub-pixel position and is blended with the ones before it.
    ///
    /// This is what turns the stochastic parts of the renderer from
    /// noise into detail — the sampled lights above, the dithered
    /// contact-shadow ray, the reduced shading rate. It costs one
    /// full-screen pass and one history texture, and it needs
    /// `compute_shading`.

    /// Which temporal technique resolves the frame (#481, #536).
    ///
    /// See [`UpscaleTechnique`](crate::quality::UpscaleTechnique) for
    /// why this is an enum dispatched by value rather than a trait
    /// object, and what contract every technique owes.
    ///
    /// 🔴 The numbers are serialised into user projects and are
    /// therefore append-only. Reordering them would silently change
    /// what an existing file means — the same class of breakage as
    /// renaming a component.
    #[serde(default = "default_upscale")]
    #[reflect(group = "Temporal", choices = UPSCALE_CHOICES)]
    pub upscale: u32,

    /// How much smaller than the window the scene is RENDERED, as a
    /// percentage of the output's width (#481, step 4).
    ///
    /// 100 renders at the window's size and the upscaler resolves
    /// without reconstructing — which is what every capture so far was
    /// taken at, and the configuration the transliteration was
    /// validated in. Below 100 the raster, the depth buffer, the
    /// visibility buffer and the shading all shrink; only the resolve's
    /// output and the tonemap stay at the window.
    ///
    /// 🔴 **This is the whole performance argument.** The shading pass
    /// costs what it costs per PIXEL — dropping to 67 % of the width is
    /// 44 % of the pixels — and everything before it shrinks with it.
    /// Nothing else in this settings file moves the frame time by that
    /// much.
    ///
    /// ⚠️ Ignored unless the technique upscales: `None` and `TAA` both
    /// resolve at render resolution and have nothing to reconstruct
    /// with, so a scale under 100 there would be a smaller image blown
    /// up by the blit — softer for no gain, which is the classic way
    /// this setting gets a bad reputation.
    #[serde(default = "default_render_scale")]
    #[reflect(
        group = "Temporal",
        choices = RENDER_SCALE_CHOICES,
        shown_when = UPSCALES_WHEN
    )]
    pub render_scale: u32,

    /// How hard the finished image is sharpened, 0..=100 (#481, step 5).
    ///
    /// RCAS — a single full-screen pass at the very end of the frame,
    /// after the curve. Reconstruction is soft by construction: the
    /// resolve builds each output pixel out of samples that landed NEAR
    /// it rather than on it, and a weighted average of neighbours is a
    /// low-pass filter however good the weights are. Every shipping
    /// upscaler ends in this pass.
    ///
    /// 🔴 **Not optional polish when `render_scale` is below 100.**
    /// Leaving it at zero there is how an upscaler earns the verdict
    /// *"we tried it, it looked worse, we turned it off"* — the frame
    /// time is won and the image is the thing everyone remembers.
    /// 60 is a reasonable starting amount; the value is judged on a
    /// screen, not in a test.
    ///
    /// ⚠️ Deliberately NOT gated on the technique. A native frame may
    /// want a little, and a control that turns itself off when the
    /// upscaler changes is worse than one that stays put.
    #[serde(default = "default_sharpening")]
    #[reflect(group = "Temporal")]
    pub sharpening: u32,

    /// Samples the filter takes along the long axis of a texture
    /// footprint, 1..=16.
    ///
    /// 🔴 The setting that improves a floor, and it is not the mip bias.
    /// A surface seen at a grazing angle covers a footprint that is long
    /// and thin, and an ordinary filter has one level for it: it takes
    /// the LONG axis, picks a level that would not alias there, and
    /// blurs the short axis by the same amount. That is why a tiled
    /// floor softens towards the horizon while a wall facing the camera
    /// stays sharp.
    ///
    /// ⚠️ It costs bandwidth, not arithmetic: more samples per fetch on
    /// exactly the surfaces that already cover the most pixels. On a
    /// handheld measured as bandwidth-bound that is the expensive kind,
    /// so 1 is the default and the number is chosen by looking at a
    /// floor and at a capture, not by picking the largest.
    // 🔴 Its OWN group, and not "Shading". The panel opens one egui
    // Grid per RUN of fields sharing a group name, so a second
    // "Shading" run after "Temporal" opened a second grid with the same
    // id — which egui reports on screen as "First use of Grid ID ..." in
    // red. Renaming beats moving the field: this is panel metadata, so
    // it carries no data risk, and anisotropy is texture filtering
    // rather than shading anyway.
    #[serde(default = "default_anisotropy")]
    #[reflect(group = "Texture filtering", choices = ANISOTROPY_CHOICES)]
    pub anisotropy: u32,

    /// Whether the surface waits for the vblank before presenting.
    ///
    /// The one graphics option every game ships and this engine only had
    /// as `KOOCH_PRESENT_MODE` — so a game built with Kóoch could not
    /// offer a vsync toggle without asking the player to set an
    /// environment variable.
    ///
    /// 🔴 A `bool` and not a mode. wgpu has six presentation modes and
    /// this engine picks between two of them; a field that serialised
    /// the other four would be offering settings the surface code does
    /// not implement. Growing to mailbox is a new field with its own
    /// default, not a renumbering of this one — the same rule
    /// [`Self::upscale`] carries.
    ///
    /// ⚠️ Off makes the frame-time readout mean something and makes the
    /// machine draw frames nobody sees. It is a measurement setting, and
    /// `KOOCH_PRESENT_MODE=novsync` turns it off for one run without
    /// touching the file.
    #[serde(default = "default_vsync")]
    #[reflect(group = "Presentation")]
    pub vsync: bool,

    /// Where the window sits between a rectangle on a desktop and the
    /// whole screen: 0 windowed, 1 borderless, 2 fullscreen,
    /// 3 exclusive.
    ///
    /// The entry directly above vsync in every graphics options menu
    /// ever shipped, and the engine could not express it — a window had
    /// a title and a size and nothing else.
    ///
    /// 🔴 `Borderless` is a WINDOW without a border, still at the
    /// project's `width x height`. `Fullscreen` covers the monitor at
    /// the monitor's **current** mode. `Exclusive` asks the display to
    /// **change mode** — the only one that alters the output resolution,
    /// implemented on Windows and X11 and **ignored by winit on
    /// Wayland**, where it is degraded to `Fullscreen` rather than left
    /// to change nothing.
    ///
    /// 🔴 The numbers are serialised into user projects and are
    /// therefore append-only, the same rule [`Self::upscale`] carries.
    /// An unrecognised one is windowed — a file from a newer engine has
    /// to open in the mode that always works rather than take the
    /// display.
    #[serde(default = "default_window_mode")]
    #[reflect(group = "Presentation", choices = WINDOW_MODE_CHOICES)]
    pub window_mode: u32,
}

/// 🔴 Serialised into user projects, so append-only.
const WINDOW_MODE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Windowed — a normal window at the project's size",
        value: 0,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Borderless — the same size, no title bar",
        value: 1,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Fullscreen — the monitor, at its current mode",
        value: 2,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Exclusive — changes the display's mode (not on Wayland)",
        value: 3,
    },
];

/// The powers of two hardware implements. Anything between them is
/// rounded down by the driver, so offering 3 would be offering 2 under
/// another name.
const ANISOTROPY_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Off — one sample",
        value: 1,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "2x",
        value: 2,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "4x",
        value: 4,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "8x",
        value: 8,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "16x",
        value: 16,
    },
];

/// Vsync on. What a player wants, and what an editor wants: uncapped
/// costs a GPU to draw frames nobody sees. Measuring is the exception
/// and `KOOCH_PRESENT_MODE=novsync` is how it asks.
fn default_vsync() -> bool {
    true
}

/// Windowed. The mode that works on every platform and every
/// compositor, and the one an author has to opt out of rather than into
/// — a file that predates this field must not take the display.
fn default_window_mode() -> u32 {
    0
}

/// Off, like every other quality setting in this file: it costs
/// bandwidth on the surfaces that already cover the most pixels, and a
/// project that never asked for that should not pay it.
fn default_anisotropy() -> u32 {
    1
}

/// The techniques the inspector offers.
///
/// 🎯 Both transliterations are judged the same way, and the menu is
/// how: set the render scale to Native and either of them becomes a
/// pure resolve, running against the engine's own on the same frames.
/// A port that is wrong shows as a difference from a known-good image
/// rather than as a vague softness.
///
/// SGSR 2 is two passes and cheap; FSR 3.1 is six and is not, and that
/// is now measured rather than expected. On the settled OneXFly at 10 W,
/// through the Steam launch path: **SGSR 2 1.868 ms, FSR 3.1 11.682**,
/// against a whole-frame budget of 13.9. The upscaler alone takes 84 %
/// of it, and 81 % of the upscaler is its accumulation pass.
///
/// 🔴 So the label says "desktop", because a menu that offers a
/// handheld user a technique which cannot fit in their frame is a trap
/// dressed as a choice. It stays offered because on a part with headroom
/// it is the better image — feature locking, reactivity, and a
/// disocclusion test that is exact rather than approximate.
const UPSCALE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "None — no history, no jitter",
        value: 0,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "TAA — the engine's own resolve",
        value: 1,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "SGSR 2 — Qualcomm's, transliterated",
        value: 2,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "FSR 3.1 — AMD's, transliterated (desktop: 6x SGSR 2)",
        value: 3,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "DLSS — NVIDIA's, linked (NVIDIA + Vulkan only)",
        value: 4,
    },
];

/// No resolve — what every capture before #481 was taken against. A
/// temporal technique rewrites every pixel of the image, which is not
/// something to adopt for a project that never asked for it.
fn default_upscale() -> u32 {
    0
}

/// 🔴 `render_scale` is only shown for techniques that reconstruct.
///
/// It is already IGNORED for the others — `RenderSettings::temporal`
/// forces it to 100 unless the technique upscales — but a control that
/// silently does nothing is worse than an absent one: it invites the
/// reading that the setting was tried and did not help. Reported by the
/// owner, who set it under TAA and reasonably expected it to apply.
///
/// The values are the enum's, and they are append-only for the same
/// reason the choices are: they live in user projects.
static UPSCALES_WHEN: kooch_ecs::reflect::FieldCondition = kooch_ecs::reflect::FieldCondition {
    field: "upscale",
    // Sgsr2, Fsr3, Dlss.
    values: &[2, 3, 4],
};

/// AMD's preset ladder, by the name each ratio is known under, because
/// "Quality" is what a player recognises and 67 % is what it means.
const RENDER_SCALE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Native — 100 %, no reconstruction",
        value: 100,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Quality — 67 % (1.5x)",
        value: 67,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Balanced — 59 % (1.7x)",
        value: 59,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Performance — 50 % (2x)",
        value: 50,
    },
];

fn default_render_scale() -> u32 {
    100
}

/// One shadow texel per screen pixel, which is Epic's ask and the
/// ceiling of the list.
///
/// # 🔴 The cascades are finer than this, and the setting does not
/// reach them
///
/// A cascade hands 2048 texels to a slice of the frustum whatever the
/// screen asked for, so it spends roughly TWICE this resolution at
/// every distance:
///
/// | distance | cascade | pages @100 |
/// |---|---|---|
/// | 5 m | 0.8 cm | 1.0 cm |
/// | 10 m | 0.8 cm | 2.0 cm |
/// | 40 m | 4.1 cm | 8.0 cm |
/// | 80 m | 8.7 cm | 16.0 cm |
///
/// The list used to go to 400 to close that gap. It no longer does, on
/// purpose: a quality option whose maximum is not the maximum reads as
/// broken, and reaching for 200 % to match the technique being replaced
/// is an admission that the number is anchored wrong, not a setting
/// anybody would find. What closes the gap honestly is a filter — the
/// cascade path runs a blocker search and a wide kernel while the page
/// path samples a raw 2x2 — and that is a different piece of work.
///
/// `the_paged_shadow_resolves_like_a_cascade` pins the table so the gap
/// is a measured number rather than an impression.
/// 1 — the bilinear the retired cube path had in hardware. Softness is
/// paid per light per pixel, so it is opted into, not defaulted.
fn default_shadow_softness() -> u32 {
    1
}

fn default_shadow_density() -> u32 {
    100
}

/// 8: on a 1080p screen, a light whose whole reach projects to a
/// 16-pixel blob. Its shadow would be a page nobody can see.
fn default_shadow_min_pixels() -> u32 {
    8
}

/// One pixel, which is what `MeshletLodSettings::default` has always
/// been. Changing the default would change every existing project's
/// geometry on the frame this landed.
/// What `inti_pbr.wgsl` held as a constant before the settings could
/// reach it.
fn default_shadow_normal_bias() -> f32 {
    1.8
}

/// Likewise.
fn default_shadow_depth_bias() -> f32 {
    0.02
}

/// 🔴 Zero — OFF — because turning it on is a behaviour change and the
/// value that is right has not been measured yet. The same reasoning as
/// `shadow_light_reach`: a cap chosen from arithmetic rather than from a
/// picture is a number nobody validated.
fn default_shadow_bias_max() -> f32 {
    0.0
}

/// 🔴 NOT zero, unlike `default_shadow_bias_max`. That one is a distance
/// in metres whose right value depends on the scene; this one is an
/// ANGLE the receiver's own geometry fixes. 4 is `tan 76°`, past which a
/// surface is nearly edge-on to the sun and the extrapolation stops
/// being a correction and starts being a divergence.
fn default_shadow_bias_slope() -> f32 {
    4.0
}

/// Half a page each way, so a receiver in the outer half of its page
/// already asks for the neighbour. Epic size their dilation from a
/// border in texels; half a page is the coarsest version of the same
/// idea and the one whose cost is easiest to read off the panel.
fn default_shadow_page_halo() -> f32 {
    0.5
}

/// A project written before the switch existed had the bound ON, so
/// absence has to read as `true` or loading an old settings file would
/// silently change what the frame draws.
fn default_true() -> bool {
    true
}

fn default_meshlet_lod_error() -> f32 {
    1.0
}

/// 🔴 Zero — every instance the frustum holds is drawn, which is what
/// shipped. Turning it on hides geometry, and what a non-zero value
/// hides is a judgement the author makes, not a default anyone
/// inherits.
fn default_meshlet_min_pixels() -> f32 {
    0.0
}

/// On. The two-level shape draws exactly the same meshlets as the
/// rectangle did — it only stops dispatching the ones that were never
/// going to survive a bounds check.
fn default_meshlet_two_level() -> bool {
    true
}

/// 🔴 Zero, because turning it on is a behaviour change and nothing has
/// measured what it costs. A threshold chosen from a whiteboard is how
/// `DEFAULT_PAGES` came to sit at half of Epic's for a year.
fn default_shadow_light_reach() -> u32 {
    0
}

/// 🔴 Off. The cascades are what every scene in the project was authored
/// against, and a technique that replaces them cannot become the default
/// on the frame it first renders.
/// 🔴 **ON since 2026-08-24.** It was off because every scene in the
/// project had been authored against the cascades — a compatibility
/// warning, never a claim that the cascades were better. On the OneXFly,
/// `many_lights` (100 point lights) at 10 W:
///
/// | | cascades era | pages, today |
/// |---|---|---|
/// | frame | 91.01 ms | **13.88 ms** |
/// | GPU | ~69.6 ms | **12.2 ms** |
/// | FPS | 11.0 | **72** |
///
/// 12.2 ms against a 13.9 ms handheld budget, with the frame limited by
/// the compositor rather than by the GPU for the first time. What got it
/// there is #952: marking per cluster (Olsson §III) and clipping page
/// triangles in hardware instead of discarding them.
///
/// ⚠️ A project that wants the cascades still has the setting. This
/// changes what a scene renders with by default, so a scene authored
/// against cascades renders differently the first time it is opened —
/// that is the cost, and it is paid once.
fn default_virtual_shadows() -> bool {
    true
}

fn default_shadow_pool_pages() -> u32 {
    kooch_render_pool_default()
}

/// Indirection so the default and the pool agree without this module
/// reaching into the shadow tree for a constant it would then have to
/// keep in step by hand.
fn kooch_render_pool_default() -> u32 {
    crate::shadow::pages::pool::DEFAULT_PAGES
}

/// Powers of two, because the atlas is a square grid of pages and the
/// labels are the only place the megabytes are ever stated.
const SHADOW_POOL_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "1024 pages — 64 MiB",
        value: 1024,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "2048 pages — 128 MiB, under today's fixed 152",
        value: 2048,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "4096 pages — 256 MiB, Epic's default pool",
        value: 4096,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "6144 pages — 384 MiB, Epic's open-world figure",
        value: 6144,
    },
];

/// 🔴 A control that silently does nothing is worse than an absent one:
/// it invites the reading that the setting was tried and did not help.
/// The two techniques do not share knobs — the page reader has no
/// blocker search for `sun_softness` to widen and no splits for
/// `shadow_first_cascade_distance` to place — so each side's fields
/// appear only in its own mode.
///
/// ⚠️ What stays visible in BOTH is deliberate and was checked in the
/// code rather than assumed: `shadow_cascade_texels` sizes every layer
/// of the shared atlas including the spot lights', `point_shadows` is
/// the cube budget the local lights still use, and `shadows_enabled`
/// gates the whole pass.
const PAGES_ON: kooch_ecs::reflect::FieldCondition = kooch_ecs::reflect::FieldCondition {
    field: "virtual_shadows",
    values: &[1],
};

/// The other side of [`PAGES_ON`].
const PAGES_OFF: kooch_ecs::reflect::FieldCondition = kooch_ecs::reflect::FieldCondition {
    field: "virtual_shadows",
    values: &[0],
};

/// The density options, so a test can assert that the top of the list
/// is the default rather than trusting a comment that says so.
pub fn shadow_density_choices() -> &'static [kooch_ecs::reflect::FieldChoice] {
    SHADOW_DENSITY_CHOICES
}

/// 🔴 100 % is a REFERENCE, not a maximum, and the list used to stop
/// there — "nobody reaches for a graphics option hoping to find
/// something above maximum".
///
/// That reasoning holds for a quality tier and is wrong for this
/// number. 100 % means one shadow texel per screen pixel measured in
/// the SUN's plane, and a texel lands square only on a surface facing
/// the sun. On one tilted by θ it lands as a rectangle `1 / cos θ`
/// long — at 11° of elevation, five times — so the receiver is already
/// under one texel per pixel with the control at its old ceiling. The
/// setting was pinned on the wrong side of the case that needs it.
///
/// Epic's is a signed LOD bias for the same reason, and it goes
/// negative: `r.Shadow.Virtual.ResolutionLodBiasDirectional`, where
/// *"lowering the value by -1 doubles the resolution of shadows with
/// the associated performance tradeoffs"*. They name the artefact
/// **projective aliasing** — *"when a shadow is cast on a surface
/// almost parallel to the light direction"*.
///
/// ⚠️ The tradeoff is not gentle: a level is a quarter of the pages, so
/// 200 % is 4x and 400 % is 16x. The pass already clamped to 400 — this
/// list was the only thing holding the ceiling at 100. Read the pool's
/// `slice used` on the performance panel before leaving one of the top
/// two on.
///
/// The values below 100 are not all powers of two, and 75 is the
/// interesting one. A page's level is `floor(log2(...))`, so 75 % does
/// NOT make every texel three quarters the size — it moves the RADIUS
/// at which the chain steps to the next level. Part of the scene lands
/// on the level 100 % would have picked and part on the level 50 %
/// would have, and the page count falls somewhere between the two. The
/// rings move; they do not blur.
const SHADOW_DENSITY_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Quarter — 25 %, a sixteenth of the pages",
        value: 25,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Half — 50 %, a quarter of the pages",
        value: 50,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Three quarters — 75 %, the level steps move outward",
        value: 75,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Full — 100 %, one texel per screen pixel",
        value: 100,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Double — 200 %, one level finer · 4x the pages",
        value: 200,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Quadruple — 400 %, two levels finer · 16x the pages",
        value: 400,
    },
];

/// The footprint widths on offer. `(width + 1)²` is the loads per
/// light per pixel, which is why the list is short and the wide end is
/// named after its bill.
/// The distance gate, in multiples of a light's own range.
///
/// The steps are what the arithmetic makes meaningful rather than round
/// numbers: a lamp contributes nothing past its range by definition, so
/// 1 is the aggressive end and 8 is "only cut what is plainly pointless".
/// 0.01 to 8 pixels, in hundredths.
///
/// The floor is deliberately not zero — see `meshlet_lod_error`. The
/// ceiling is where a mesh is already down to its root cluster on
/// anything but a full-screen object, so past it the control stops
/// doing anything.
/// Zero is off; 64 pixels is already a quarter of a 256-tall viewport,
/// past which the control stops being a reach and starts being a
/// deletion.
const MESHLET_MIN_PIXELS_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.0,
    max: 64.0,
    step: 0.5,
};

const SHADOW_NORMAL_BIAS_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.0,
    max: 8.0,
    step: 0.05,
};

const SHADOW_DEPTH_BIAS_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.0,
    max: 0.5,
    step: 0.001,
};

/// Up to a metre. Past that it is not a cap on anything the chain
/// produces below level 14.
const SHADOW_BIAS_MAX_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.0,
    max: 1.0,
    step: 0.005,
};

/// 0 is one depth for every tap — what shipped. 8 is `tan 83°`, past
/// which the clamp is not clamping anything the geometry reaches.
const SHADOW_BIAS_SLOPE_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.0,
    max: 8.0,
    step: 0.1,
};

const MESHLET_LOD_ERROR_RANGE: kooch_ecs::reflect::FieldRange = kooch_ecs::reflect::FieldRange {
    min: 0.01,
    max: 8.0,
    step: 0.01,
};

const SHADOW_LIGHT_REACH_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Off — distance never gates",
        value: 0,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "1x range — aggressive",
        value: 1,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "2x range",
        value: 2,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "4x range",
        value: 4,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "8x range — conservative",
        value: 8,
    },
];

const SHADOW_MIN_PIXELS_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Off — every light casts",
        value: 0,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "4 px — gate only the invisible",
        value: 4,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "8 px — the default",
        value: 8,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "16 px",
        value: 16,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "32 px — distant lamps go shadowless",
        value: 32,
    },
];

const SHADOW_SOFTNESS_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Sharp — bilinear, 4 taps, the cube path's look",
        value: 1,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Soft — 2 texels, 9 taps",
        value: 2,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Softer — 3 texels, 16 taps",
        value: 3,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Softest — 5 texels, 36 taps: measure before shipping",
        value: 5,
    },
];

/// No sharpening, for the reason no resolve is the default: this
/// rewrites every pixel of a finished image, and a project that never
/// mentioned it did not ask for that.
///
/// ⚠️ The engine's own capture project is not that project — a scale
/// below 100 without this is half the change, and the half that gets
/// judged.
fn default_sharpening() -> u32 {
    0
}

/// The two rates that exist. Quarter rate is deliberately absent: at
/// 4x4 the upsample's guide stops being able to reconstruct a
/// silhouette, which is a different technique rather than a bigger
/// constant.
const SHADING_RATE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Full — one sample per pixel",
        value: 1,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Half — one sample per 2x2 quad",
        value: 2,
    },
];

fn default_aperture() -> f32 {
    PhysicalCamera::default().aperture_f_stops
}
fn default_shutter() -> f32 {
    PhysicalCamera::default().shutter_speed_s
}
fn default_iso() -> f32 {
    PhysicalCamera::default().sensitivity_iso
}
fn default_sky() -> glam::Vec3 {
    AmbientLight::default().sky_color
}
fn default_ground() -> glam::Vec3 {
    AmbientLight::default().ground_color
}
fn default_ambient_intensity() -> f32 {
    AmbientLight::default().intensity
}
fn default_shadows_enabled() -> bool {
    ShadowSettings::default().enabled
}
fn default_shadow_distance() -> f32 {
    ShadowSettings::default().max_distance
}
fn default_cascade_texels() -> u32 {
    ShadowSettings::default().cascade_texels
}
fn default_sun_softness() -> f32 {
    ShadowSettings::default().sun_softness
}
fn default_first_cascade() -> f32 {
    ShadowSettings::default().first_cascade_distance
}
fn default_contact_steps() -> u32 {
    ContactShadowSettings::default().linear_steps
}
fn default_contact_length() -> f32 {
    ContactShadowSettings::default().length
}
fn default_point_shadows() -> u32 {
    crate::shadow::DEFAULT_POINT_SHADOWS
}
fn default_contact_dominant() -> bool {
    ContactShadowSettings::default().dominant_only
}
fn default_contact_thickness() -> f32 {
    ContactShadowSettings::default().thickness
}
/// 🔴 These four are the ENGINE's defaults, deliberately, and an
/// earlier version of this file got it wrong.
///
/// It shipped with `compute_shading` and the temporal resolve defaulting to
/// true, reasoning that a project with a settings asset has an author
/// who can see the result. What actually happened is that every
/// existing project — which has a `.rendersettings` written before
/// these fields existed, and therefore takes every one of these
/// defaults — changed shading path AND gained a temporal resolve in the
/// same build. Two variables at once is not a change anybody can
/// bisect, and the first report was "you broke the whole render".
///
/// A serde default is not a recommendation. It is what an old file
/// silently becomes, so it has to be what the engine already did:
/// fragment path, full rate, every light, no history — the shape every
/// capture before #824 was taken against. The knobs are in the
/// Inspector; turning one on is a decision, and a decision has somebody
/// looking at the screen when it is taken.
fn default_compute_shading() -> bool {
    false
}
fn default_shading_rate() -> u32 {
    crate::meshlet::ShadingRate::Full.factor()
}

impl Default for RenderSettings {
    /// The same values the engine uses with no settings asset at all —
    /// deliberately, so adding the file changes nothing until someone
    /// edits it.
    fn default() -> Self {
        let camera = PhysicalCamera::default();
        let ambient = AmbientLight::default();
        let shadows = ShadowSettings::default();
        let contact = ContactShadowSettings::default();
        Self {
            shadow_normal_bias: default_shadow_normal_bias(),
            shadow_depth_bias: default_shadow_depth_bias(),
            shadow_bias_max: default_shadow_bias_max(),
            shadow_bias_slope: default_shadow_bias_slope(),
            shadow_page_march: false,
            shadow_page_geometry: false,
            shadow_page_receiver_bound: true,
            shadow_page_halo: default_shadow_page_halo(),
            meshlet_lod_error: default_meshlet_lod_error(),
            meshlet_min_pixels: default_meshlet_min_pixels(),
            meshlet_two_level: default_meshlet_two_level(),
            aperture_f_stops: camera.aperture_f_stops,
            shutter_speed_s: camera.shutter_speed_s,
            sensitivity_iso: camera.sensitivity_iso,
            ambient_sky_color: ambient.sky_color,
            ambient_ground_color: ambient.ground_color,
            ambient_intensity: ambient.intensity,
            shadows_enabled: shadows.enabled,
            shadow_distance: shadows.max_distance,
            shadow_cascade_texels: shadows.cascade_texels,
            shadow_softness: shadows.page_softness,
            shadow_min_pixels: shadows.page_min_pixels,
            shadow_light_reach: shadows.page_light_reach,
            sun_softness: shadows.sun_softness,
            shadow_first_cascade_distance: shadows.first_cascade_distance,
            contact_shadow_steps: contact.linear_steps,
            contact_shadow_length: contact.length,
            contact_shadow_thickness: contact.thickness,
            contact_shadow_dominant: contact.dominant_only,
            point_shadows: shadows.point_shadows,
            compute_shading: default_compute_shading(),
            shading_rate: default_shading_rate(),
            upscale: 0,
            render_scale: default_render_scale(),
            shadow_density: default_shadow_density(),
            virtual_shadows: default_virtual_shadows(),
            shadow_pool_pages: default_shadow_pool_pages(),
            sharpening: default_sharpening(),
            anisotropy: default_anisotropy(),
            vsync: default_vsync(),
            window_mode: default_window_mode(),
        }
    }
}

impl RenderSettings {
    pub fn camera(&self) -> PhysicalCamera {
        PhysicalCamera {
            aperture_f_stops: self.aperture_f_stops,
            shutter_speed_s: self.shutter_speed_s,
            sensitivity_iso: self.sensitivity_iso,
        }
    }

    pub fn ambient(&self) -> AmbientLight {
        AmbientLight {
            sky_color: self.ambient_sky_color,
            ground_color: self.ambient_ground_color,
            intensity: self.ambient_intensity,
        }
    }

    /// The camera cull's LOD target, as the frame wants it.
    ///
    /// 🔴 The resource this returns already existed and the frame
    /// already read it — only the editor ever inserted it, so a shipped
    /// game found `None` and fell back to the default. The value was
    /// a live slider in the editor and unreachable everywhere else.
    pub fn meshlet_lod(&self) -> crate::meshlet::MeshletLodSettings {
        crate::meshlet::MeshletLodSettings {
            // Clamped rather than trusted: the range constrains the
            // Inspector, and a settings file is a text file anyone can
            // write a zero into. A target of zero means no level is
            // ever fine enough and the cull emits nothing at all.
            target_error_pixels: self.meshlet_lod_error.clamp(0.01, 8.0),
            // Not clamped to a floor the way the LOD target is: zero is
            // the meaningful "off", not a degenerate value.
            min_screen_pixels: self.meshlet_min_pixels.clamp(0.0, 256.0),
            two_level: self.meshlet_two_level,
        }
    }

    pub fn shadows(&self) -> ShadowSettings {
        ShadowSettings {
            page_normal_bias: self.shadow_normal_bias,
            page_depth_bias: self.shadow_depth_bias,
            page_bias_max: self.shadow_bias_max,
            page_bias_slope: self.shadow_bias_slope,
            page_march: self.shadow_page_march,
            page_geometry: self.shadow_page_geometry,
            page_receiver_bound: self.shadow_page_receiver_bound,
            page_halo: self.shadow_page_halo,
            max_distance: self.shadow_distance,
            cascade_texels: self.shadow_cascade_texels,
            enabled: self.shadows_enabled,
            sun_softness: self.sun_softness,
            first_cascade_distance: self.shadow_first_cascade_distance,
            point_shadows: crate::shadow::point_shadows_from_environment()
                .unwrap_or(self.point_shadows),
            // 🔴 `KOOCH_PAGE_MARKING=1` is a FORCE on top of the asset,
            // the way `point_shadows_from_environment` is — and it is
            // applied HERE rather than at the call site. The call site
            // version could not run at all: it sat behind a lookup of
            // `RenderSettings`, which is never a `Resources` value, so
            // the early return fired first and the variable was never
            // consulted.
            virtual_pages: self.virtual_shadows
                || crate::shadow::pages::mark::enabled_by_environment(),
            page_density: self.shadow_density,
            pool_pages: self.shadow_pool_pages,
            page_softness: self.shadow_softness,
            page_min_pixels: self.shadow_min_pixels,
            page_light_reach: self.shadow_light_reach,
        }
    }

    /// The author's contact shadows, with `KOOCH_CONTACT_SHADOW_STEPS`
    /// applied on top — see [`crate::contact_shadow::steps_from_environment`]
    /// for why the variable outranks the asset.
    pub fn contact_shadows(&self) -> ContactShadowSettings {
        ContactShadowSettings {
            linear_steps: crate::contact_shadow::steps_from_environment()
                .unwrap_or(self.contact_shadow_steps),
            length: self.contact_shadow_length,
            thickness: self.contact_shadow_thickness,
            dominant_only: crate::contact_shadow::dominant_from_environment()
                .unwrap_or(self.contact_shadow_dominant),
        }
    }

    /// What the frame is allowed to spend, with any `KOOCH_*` override
    /// applied on top — see [`crate::quality`] for why the variable
    /// outranks the asset.
    pub fn shading(&self) -> crate::quality::ShadingSettings {
        crate::quality::ShadingSettings::from_asset(
            self.compute_shading,
            crate::meshlet::ShadingRate::from_factor(self.shading_rate),
            self.anisotropy.min(u32::from(u16::MAX)) as u16,
        )
    }

    /// 🔴 Gated on the shading path, not merely documented as needing
    /// it. The resolve lives in the compute path's HDR chain, so asking
    /// for it on the fragment path would leave the jitter on with
    /// nothing to integrate it — a frame that shimmers, which reads as
    /// TAA being broken rather than absent.
    pub fn temporal(&self) -> crate::quality::TemporalSettings {
        let technique = if self.shading().compute {
            self.technique()
        } else {
            crate::quality::UpscaleTechnique::None
        };
        // 🔴 The effective compute flag, not the field: `KOOCH_COMPUTE_SHADING`
        // can turn the path off for a capture run, and a scale that
        // survived that override would take the frame down with it.
        crate::quality::TemporalSettings::new(
            technique,
            self.render_scale,
            self.sharpening,
            self.shading().compute,
        )
    }

    /// How frames reach the display, with `KOOCH_PRESENT_MODE` applied
    /// on top — see [`crate::quality`] for why the variable outranks the
    /// asset.
    pub fn presentation(&self) -> crate::quality::Presentation {
        crate::quality::Presentation::from_asset(self.vsync)
    }

    /// Where the window sits, with `KOOCH_WINDOW_MODE` applied on top.
    ///
    /// A `kooch_core` type rather than one of ours: it is authored here
    /// and applied by `kooch_window`, and those two crates know only
    /// `kooch_core` and each other's absence.
    pub fn window_mode(&self) -> kooch_core::window_mode::WindowMode {
        kooch_core::window_mode::WindowMode::from_asset(self.window_mode)
    }

    /// The technique this file asks for.
    pub fn technique(&self) -> crate::quality::UpscaleTechnique {
        crate::quality::UpscaleTechnique::from_asset(self.upscale)
    }

    /// Publishes into the `Resources` the shading model already reads.
    ///
    /// The indirection is the point: `inti_pbr.wgsl` and `GpuLights`
    /// never learn what an asset is, so a game that sets `Exposure`
    /// directly keeps working and a headless test needs no file.
    pub fn apply(&self, resources: &mut Resources) {
        resources.insert(self.presentation());
        resources.insert(self.window_mode());
        resources.insert(Exposure::from_physical(self.camera()));
        resources.insert(self.ambient());
        resources.insert(self.shadows());
        resources.insert(self.contact_shadows());
        let shading = self.shading();
        resources.insert(shading);
        resources.insert(self.temporal());
        resources.insert(self.meshlet_lod());
    }
}

/// Reads a `.rendersettings` file.
#[derive(Debug, Default, Clone, Copy)]
pub struct RenderSettingsLoader;

impl AssetLoader<RenderSettings> for RenderSettingsLoader {
    fn extensions(&self) -> &[&'static str] {
        &[RENDER_SETTINGS_EXTENSION]
    }

    fn load(&self, bytes: &[u8], _ctx: &mut LoadContext<'_>) -> AssetResult<RenderSettings> {
        let text = std::str::from_utf8(bytes).map_err(|e| AssetError::Loader(Box::new(e)))?;
        // Every field has a serde default, so a file with one line in it
        // is valid and everything else stays at the engine's value. A
        // settings file should never fail to load because it is old.
        ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(e)))
    }
}

kooch_ecs::register_reflected_asset!(RenderSettings, RenderSettingsLoader);

/// Serialises settings for writing.
pub fn to_ron(settings: &RenderSettings) -> Result<String, ron::Error> {
    ron::ser::to_string_pretty(settings, ron::ser::PrettyConfig::default())
}

#[cfg(test)]
mod tests;

/// Finds the project's settings asset, loads it, and publishes the
/// values the shading model reads.
///
/// Runs every frame, and that is deliberate rather than lazy: the asset
/// is reloaded in place when it is saved (#728), so the only way to
/// notice an edit without polling would be a change signal the asset
/// system does not have. The cost is a hash lookup and, when something
/// actually differs, two `Resources` inserts.
///
/// **A project with no settings asset is the normal case**, not an
/// error: the engine's defaults already apply, and this returns without
/// touching anything.
///
/// Discovery is by type rather than by path — one `.rendersettings` per
/// project, found wherever the author put it. Two of them is ambiguous
/// and warned about once, taking the first in scan order so the scene
/// still renders.
pub fn apply_render_settings_system(resources: &mut Resources) {
    let Some(guid) = find_settings_guid(resources) else {
        return;
    };
    let Some(handle) =
        kooch_ecs::reflect::asset_registry::load_handle::<RenderSettings>(resources, guid)
    else {
        return;
    };
    let Some(settings) = resources
        .get::<kooch_core::assets::Assets<RenderSettings>>()
        .and_then(|assets| assets.get(handle))
        .copied()
    else {
        return;
    };

    // Only write when something changed. Inserting unconditionally would
    // be correct and would also mean every frame reports the resource as
    // freshly set, which any future change detection would believe.
    let exposure = Exposure::from_physical(settings.camera());
    let ambient = settings.ambient();
    let shadows = settings.shadows();
    let contact = settings.contact_shadows();
    let shading = settings.shading();
    let temporal = without_missing_dlss(settings.temporal(), resources);
    let meshlet_lod = settings.meshlet_lod();
    let presentation = settings.presentation();
    let window_mode = settings.window_mode();
    let stale = resources.get::<crate::quality::Presentation>() != Some(&presentation)
        || resources.get::<kooch_core::window_mode::WindowMode>() != Some(&window_mode)
        || resources.get::<Exposure>() != Some(&exposure)
        || resources.get::<AmbientLight>() != Some(&ambient)
        || resources.get::<ShadowSettings>() != Some(&shadows)
        || resources.get::<ContactShadowSettings>() != Some(&contact)
        || resources.get::<crate::quality::ShadingSettings>() != Some(&shading)
        || resources.get::<crate::quality::TemporalSettings>() != Some(&temporal)
        || resources.get::<crate::meshlet::MeshletLodSettings>() != Some(&meshlet_lod);
    if stale {
        settings.apply(resources);
        // 🔴 After `apply`, which inserts the technique the FILE asked
        // for. A project authored on a machine with DLSS is opened on
        // one without, and the value it wrote is still the right thing
        // to keep in the asset — what must not survive is the engine
        // then trying to run it.
        resources.insert(temporal);
        tracing::debug!(
            target: "kooch_render::settings",
            ev100 = exposure.ev100,
            "render settings applied",
        );
    }
}

/// Downgrades DLSS to the engine's own resolve when this build, or this
/// adapter, cannot run it (#536).
///
/// 🔴 The asset is left alone. `upscale = 4` is a statement about what
/// the project wants, and a settings file that quietly rewrote itself on
/// the developer's laptop would ship the wrong value to the machine that
/// could have honoured it.
///
/// The scale goes back to 100 with the technique: TAA resolves, it does
/// not reconstruct, and leaving a half-sized render behind would trade
/// the missing upscaler for a blurry frame nobody asked for.
fn without_missing_dlss(
    mut temporal: crate::quality::TemporalSettings,
    resources: &Resources,
) -> crate::quality::TemporalSettings {
    if temporal.technique != crate::quality::UpscaleTechnique::Dlss {
        return temporal;
    }
    let available = resources
        .get::<kooch_core::gpu::DlssRuntime>()
        .is_some_and(|runtime| runtime.support.super_resolution);
    if available {
        return temporal;
    }
    warn_once_about_missing_dlss();
    temporal.technique = crate::quality::UpscaleTechnique::Taa;
    temporal.render_scale = 100;
    temporal
}

/// Says it once. The condition cannot change within a session — neither
/// the adapter nor the linked SDK does — so a line per frame would be a
/// log nobody reads.
fn warn_once_about_missing_dlss() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        tracing::warn!(
            target: "kooch_render::settings",
            "the project asks for DLSS and this build or adapter has none; \
             resolving with the engine's own TAA at full resolution instead",
        );
    });
}

/// The guid of the project's settings asset, if it has one.
fn find_settings_guid(resources: &Resources) -> Option<kooch_core::Guid> {
    let db = resources.get::<kooch_core::asset_database::AssetDatabase>()?;
    let type_name = std::any::type_name::<RenderSettings>();
    let mut found = db.entries_of_type(type_name);
    let first = found.next()?;
    if found.next().is_some() {
        tracing::warn!(
            target: "kooch_render::settings",
            "more than one .rendersettings in the project; using the first found. \
             Settings are per project, so the others do nothing.",
        );
    }
    Some(first.0)
}
