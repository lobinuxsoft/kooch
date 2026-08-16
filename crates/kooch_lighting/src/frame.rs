//! Per-frame lighting constants: what the scene is exposed at, and
//! what light arrives from nowhere in particular.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Hemisphere ambient — the stand-in for image-based lighting until
/// #450 lands a real probe.
///
/// Not cosmetic. With no ambient term a metal has no environment to
/// reflect, so every metallic surface not facing a light renders pure
/// black: correct for the model, and indistinguishable from a bug to
/// whoever is looking at it.
///
/// Insert one into [`Resources`](kooch_core::resource::Resources) to
/// override; absent, the default below is used.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AmbientLight {
    /// Linear RGB arriving from world up.
    pub sky_color: Vec3,
    /// Linear RGB arriving from world down — bounce, not sky.
    pub ground_color: Vec3,
    /// Illuminance in lux, on the same scale as a `DirectionalLight`.
    pub intensity: f32,
}

impl Default for AmbientLight {
    /// An overcast-ish sky over neutral ground, at roughly 3 % of the
    /// 10 000 lux a default `DirectionalLight` puts out. Enough to read
    /// shape in shadow, far too little to be mistaken for a key light.
    fn default() -> Self {
        Self {
            sky_color: Vec3::new(0.4, 0.55, 0.75),
            ground_color: Vec3::new(0.2, 0.18, 0.15),
            intensity: 300.0,
        }
    }
}

/// Camera exposure, in the photographic EV100 scale.
///
/// The lights carry physical units — a `DirectionalLight` defaults to
/// 10 000 lux — so without an exposure step every channel clips to
/// white and the shading model looks broken rather than unexposed.
/// This is the fixed stand-in; #254 owns auto exposure, which stops
/// being cosmetic at planetary scale where a sunlit surface and the
/// night side differ by orders of magnitude.
///
/// # Prefer [`PhysicalCamera`]
///
/// `EV100 = 9.7` is a correct number and an unusable control: nothing
/// about it says which way is brighter or how much a step is worth.
/// `f/16, 1/125 s, ISO 100` says the same thing to anyone who has held
/// a camera. [`PhysicalCamera::ev100`] converts.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Exposure {
    pub ev100: f32,
}

impl Default for Exposure {
    /// Whatever [`PhysicalCamera::default`] works out to — one source of
    /// truth rather than a bare number that has to be kept in step with
    /// the camera settings that are supposed to explain it.
    ///
    /// It lands near 9.9, which is close to Bevy's 9.7. Theirs is not
    /// "sunny 16" despite how it is often described; they calibrated it
    /// to match Blender's implicit exposure. A quarter of a stop apart
    /// means a scene authored against their numbers reads the same here.
    fn default() -> Self {
        Self::from_physical(PhysicalCamera::default())
    }
}

impl Exposure {
    /// The multiplier the shader applies to radiance before tonemapping.
    ///
    /// `1 / (2^EV100 × 1.2)`: the 1.2 is the standard reflected-light
    /// meter calibration constant, not a fudge factor.
    pub fn multiplier(&self) -> f32 {
        1.0 / (2.0f32.powf(self.ev100) * 1.2)
    }

    /// Exposure for a real camera's settings.
    pub fn from_physical(camera: PhysicalCamera) -> Self {
        Self {
            ev100: camera.ev100(),
        }
    }
}

/// A real camera's settings, as the way to say how bright the scene
/// should look.
///
/// Aperture, shutter and ISO are three numbers a person can reason
/// about — open the aperture, get more light — where EV100 is one number
/// that reasons about nothing. Bevy added the same thing in 0.13 for the
/// same reason.
///
/// This is the honest half of the fix for physical light units being
/// unusable. The other halves are auto exposure (#254) and global
/// illumination (#450); until those, an author who finds the scene too
/// dark has a control that behaves the way they expect.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PhysicalCamera {
    /// f-stop. Lower is a wider aperture and a brighter image: f/1.4
    /// gathers four times the light of f/2.8.
    pub aperture_f_stops: f32,
    /// Shutter time in seconds. `1.0 / 125.0` is a typical handheld
    /// exposure; longer is brighter.
    pub shutter_speed_s: f32,
    /// Film speed. Higher is brighter, and in a real camera noisier —
    /// here it is brightness only.
    pub sensitivity_iso: f32,
}

impl Default for PhysicalCamera {
    /// f/2.8 at 1/125 s, ISO 100 — EV100 ≈ 9.9.
    ///
    /// A middle setting rather than a real situation: bright enough that
    /// a default `DirectionalLight` does not clip and dim enough that a
    /// punctual light is visible. Neither of those is a photographic
    /// fact; they are what this renderer needs while it has no global
    /// illumination, and the presets below are the real situations.
    fn default() -> Self {
        Self {
            aperture_f_stops: 2.8,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }
}

impl PhysicalCamera {
    /// Bright sun outdoors: f/16, 1/125 s, ISO 100 — "sunny 16",
    /// EV100 ≈ 15.
    ///
    /// Pair it with `lux::DIRECT_SUNLIGHT` on the directional light.
    /// Used with a 10 000 lux default sun, the scene comes out dark,
    /// which is correct: 10 000 lux is ambient daylight, not sun.
    pub fn sunny() -> Self {
        Self {
            aperture_f_stops: 16.0,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }

    /// Indoors under artificial light: f/1.0, 1/125 s, ISO 100 —
    /// EV100 ≈ 7. The same settings Bevy's lighting example uses.
    ///
    /// About eight stops brighter than [`Self::sunny`], which is roughly
    /// the gap between a sunlit exterior and a lit room — the gap that
    /// makes a physically-correct bulb look like nothing.
    pub fn indoor() -> Self {
        Self {
            aperture_f_stops: 1.0,
            shutter_speed_s: 1.0 / 125.0,
            sensitivity_iso: 100.0,
        }
    }

    /// The equivalent EV100.
    ///
    /// `log2(N² / t) - log2(S / 100)`, the standard photographic
    /// relation: aperture and shutter set the exposure, sensitivity
    /// shifts the scale it is measured against.
    pub fn ev100(&self) -> f32 {
        let n = self.aperture_f_stops.max(1e-3);
        let t = self.shutter_speed_s.max(1e-9);
        let s = self.sensitivity_iso.max(1e-3);
        ((n * n) / t).log2() - (s / 100.0).log2()
    }
}

/// One cascade, as the shader reads it. Mirrors `IntiCascade` in
/// `inti_pbr.wgsl`: 96 bytes, and the stride has to stay a multiple of
/// 16 or the array indexes into the middle of the previous entry.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuCascade {
    pub view_proj: [[f32; 4]; 4],
    /// Which layer of the shadow array this cascade rendered into.
    ///
    /// Replaced a `uv_scale_bias` that packed this cascade's quadrant of
    /// a single atlas texture. The atlas existed on the belief that a
    /// dynamic index into several shadow maps needed binding arrays; it
    /// does not — `texture_depth_2d_array` is one binding and one
    /// sampler, and the layer is an ordinary argument to
    /// `textureSampleCompareLevel`. Bevy has always done it this way.
    pub layer: u32,
    /// 🔴 Three scalars on the WGSL side too, never a `vec3<u32>`: that
    /// aligns to 16 and grows every cascade by 16 bytes, which surfaces
    /// only as `min_binding_size` rejecting the pipeline.
    pub _pad_layer: [u32; 3],
    pub far_depth: f32,
    pub texel_world_size: f32,
    /// World units the `[0,1]` depth range spans, so the shader can turn
    /// a difference between two stored depths into metres. PCSS's
    /// penumbra is proportional to that distance, and a ratio with no
    /// scale is only usable through a constant that is wrong in three
    /// cascades out of four.
    pub depth_extent: f32,
    pub _pad0: f32,
}

/// How many shadow-casting spot lights one frame can carry (#777).
///
/// Four, because each one costs a layer of the shadow array and a cull
/// of its own — 2048² at `Depth32Float` is 16 MiB per spot. It is a
/// budget, not a limit of the technique: raising it is this constant and
/// a larger texture, and 13.9 ms at 10 W is what decides when.
///
/// Lights past the fourth still light the scene, they just do not cast.
/// Dropping the light itself would be a worse failure than dropping its
/// shadow, and a far more confusing one.
/// The most lights one froxel can be asked to choose (#826).
///
/// Mirrors `MAX_TILE_STRATA` in `material_pbr_compute.wgsl`, where one
/// thread runs one stratum and a 256-thread tile has up to 16 froxels to
/// serve. Past this the shader silently repeats the previous picture.
pub const MAX_LIGHT_SAMPLES: u32 = 8;

pub const MAX_SPOT_SHADOWS: usize = 4;

/// How many cascades the frame carries. Fixed because the count is baked
/// into the atlas layout — changing it is a texture change.
pub const FRAME_CASCADE_COUNT: usize = 4;

/// How many point lights can cast at once (#778).
///
/// Four, and the number is decided by **memory**, not by the technique.
/// A cube is six faces, so at the 512² this engine renders them at
/// (`Depth32Float`) each casting point light is 6 MiB — against 16 MiB
/// for a single 2048² cascade layer. Four of them add 24 MiB to a shadow
/// budget that is already 128 MiB.
///
/// ⚠️ The `max_texture_array_layers` ceiling of 256 would allow 42, and
/// that number is a red herring: 42 lights at this face size is 252 MiB
/// of depth, on a handheld sharing its memory with the rest of the
/// frame. Memory runs out first, by a wide margin.
///
/// Lights past the fourth still light the scene, they just do not cast —
/// same failure as [`MAX_SPOT_SHADOWS`], and see #778 on why the order
/// they are chosen in has to be deliberate rather than whatever the
/// query returned.
pub const MAX_POINT_SHADOWS: usize = 32;

/// What the shading model needs to sample one point light's cube (#778).
///
/// Sixteen bytes, and no matrix: `textureSampleCompareLevel` on a cube
/// array takes a **direction**, so the whole transform is a subtraction
/// from the light's position, which the shader already does.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuPointShadow {
    /// The near plane the six faces were rendered with.
    ///
    /// 🔴 This is the whole depth reconstruction. Bevy sends the
    /// lower-right 2×2 of the face projection and computes
    /// `depth = zw.x / zw.y`; with an infinite reverse-Z projection
    /// (ADR 0002) that collapses to `near / major_axis_magnitude`, so
    /// one scalar replaces their four. It is the same identity
    /// `depth_ndc_to_view_z` rests on.
    pub near: f32,
    /// Shadow-texel size **per metre of distance from the light**, the
    /// way a spot's is — a cube face is a 90° perspective, so this is
    /// `2 / size` and never involves `range`.
    pub texel_world_size: f32,
    /// World units the usable depth range spans, for the penumbra
    /// estimate. The light's range.
    pub depth_extent: f32,
    pub _pad0: f32,
}

/// Mirror of `IntiFrame` in `inti_pbr.wgsl`. 928 bytes.
///
/// `camera_position` rides here rather than in the shared camera UBO
/// because that UBO is pinned at 64 B by two bind-group layouts, and
/// widening it would ripple through paths this work has no business
/// touching. It also makes this struct the one per-view thing in an
/// otherwise per-frame binding — see [`crate::GpuLights::write_frame`]
/// for why that is safe with more than one view.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct IntiFrame {
    pub ambient_sky: [f32; 3],
    pub light_count: u32,
    pub ambient_ground: [f32; 3],
    pub exposure: f32,
    pub camera_position: [f32; 3],
    pub ambient_intensity: f32,
    /// Unit vector down the view axis. Only the cascades use it, and
    /// they need the axis rather than the radial direction: a radial
    /// distance makes every cascade boundary a sphere, which crosses in
    /// the corners of the screen before the centre.
    pub camera_forward: [f32; 3],
    pub _pad_forward: f32,
    pub cascades: [GpuCascade; FRAME_CASCADE_COUNT],
    /// One per shadow-casting spot light, in the same record the
    /// cascades use (#777).
    ///
    /// The same type on purpose: `inti_shadow_coords` already divides by
    /// `w`, which an orthographic cascade does not need and a spot's
    /// perspective does — the comment there has said "a spot light's,
    /// later" since #476. Reusing it means the bias, the Castano filter
    /// and the border clamp are the ones already ported from Bevy rather
    /// than a second copy that can drift from them.
    ///
    /// ⚠️ Bevy does NOT send a matrix here: it rebuilds the spot's basis
    /// in the shader from the light's direction and cone angle, because
    /// its light record has nowhere to put one. That is a constraint of
    /// their layout, not a better algorithm, and porting it would mean a
    /// second sampling path beside the one already ported.
    pub spot_shadows: [GpuCascade; MAX_SPOT_SHADOWS],
    /// How many entries of `spot_shadows` are live this frame.
    pub spot_shadow_count: u32,
    /// How many entries of `point_shadows` are live this frame.
    ///
    /// Rides in one of the three pad words the spot count left, the way
    /// `debug_light` rides in this struct's tail — a count is a word and
    /// there was a word.
    pub point_shadow_count: u32,
    /// Irradiance below which a light pays for the diffuse layer only
    /// (#821).
    ///
    /// The specular layer is the expensive half — GGX `D`,
    /// height-correlated Smith `V`, Schlick `F`, the multiscatter fit,
    /// and the representative point when the light has a radius — and a
    /// light reaching this pixel with a fraction of the frame's exposure
    /// spends all of it on a highlight nobody can see. With 15 lights
    /// per pixel, that is 15 of them.
    ///
    /// **0.0 keeps every light on the full model**, which is what every
    /// frame did before this existed.
    pub specular_floor: f32,
    /// How many of a froxel's punctual lights a pixel may evaluate, or
    /// **0 for all of them** — which is what every frame does.
    ///
    /// 🔴 A measuring instrument, not a feature. Three experiments have
    /// now made each light cheaper — the arithmetic (#821, 10 %), the
    /// storage fetch (#824, 6.6 %), the grid's over-listing (#820,
    /// nothing to win) — and the frame did not move. The one variable
    /// none of them touched is *how many* lights a pixel evaluates, and
    /// #820 measured that twelve to fifteen genuinely reach the surface,
    /// so clustering cannot remove them by definition.
    ///
    /// Truncating the walk is not a technique anybody would ship: it
    /// drops real light and the picture goes dark where the lights
    /// overlap. What it answers is whether the cost is proportional to
    /// that count — which decides #825 against #826, before either is
    /// built.
    ///
    /// Directional lights are not counted. They are the buffer's prefix
    /// and are not in the grid, so leaving them alone keeps the
    /// experiment about the froxel's lights and nothing else.
    pub light_limit: u32,
    /// One per shadow-casting point light (#778).
    ///
    /// 🔴 A different record from the spots', and the difference is the
    /// point of it: a cube map is sampled by DIRECTION, so there is no
    /// matrix to send and no uv to transform. What is left is the three
    /// scalars below. Reusing `GpuCascade` here would ship a 64-byte
    /// matrix per light that nothing reads, four times over.
    pub point_shadows: [GpuPointShadow; MAX_POINT_SHADOWS],
    /// 0 when nothing casts, or the atlas has not been rendered. The
    /// dummy atlas bound in that case reads as fully lit anyway; the
    /// flag skips the sampling.
    pub shadows_enabled: u32,
    /// Fraction of a split distance over which one cascade fades into
    /// the next.
    pub cascade_blend: f32,
    /// Tangent of the sun's angular RADIUS — how much wider a shadow
    /// gets per metre between blocker and receiver.
    ///
    /// An angle rather than a width, because that is what a light
    /// infinitely far away has. A width in world units would have to
    /// mean a width *at some distance*, and no code path was ever going
    /// to agree on which.
    pub sun_softness: f32,
    /// Which light the single-light debug view isolates (#743), as an
    /// index into the light buffer. Anything `>= light_count` means
    /// "none selected", including the [`NO_DEBUG_LIGHT`] default.
    ///
    /// It rides in what used to be this struct's tail padding, so the
    /// view costs no binding, no buffer and not one byte — which matters
    /// because there is no seventh bind group to put one in, and Inti's
    /// group is already full.
    pub debug_light: u32,
    /// The third row of the view matrix, so a fragment can turn its
    /// world position into a view-space depth with one dot product.
    ///
    /// A row rather than the matrix: the only thing shading needs from
    /// the camera's orientation is which slice of the froxel grid it is
    /// in, and that is `z` alone. The other three rows would be 48 bytes
    /// nothing reads.
    pub view_z_row: [f32; 4],
    /// xyz = the grid's dimensions, w = their product.
    pub cluster_dimensions: [u32; 4],
    /// xy = grid cells per pixel, zw = the logarithmic slice constants.
    pub cluster_factors: [f32; 4],
    /// How many indices the list holds, for the loop to clamp against.
    ///
    /// A frame whose lighting overflowed the list leaves later cells
    /// pointing past the end of it. Clamping renders those cells under-lit
    /// rather than reading whatever a stale index happens to name.
    pub cluster_capacity: u32,
    /// Directional lights, which the grid does not cluster: they reach
    /// every cell, so listing them per cell would say nothing. They are
    /// the first `directional_count` entries of the light buffer and the
    /// shader walks them linearly.
    pub directional_count: u32,
    /// 0 while no grid has been built — an unclustered frame walks every
    /// light the way it did before #780, which is what the headless
    /// tests and any path with no camera matrices do.
    pub clustered: u32,
    /// Count at which `MeshletDebugMode::LightsPerPixel` reads full red
    /// (#817). Rides in the word the cluster flag left, the way
    /// `debug_light` rides in this struct's tail.
    ///
    /// A uniform rather than a shader constant because the useful top of
    /// scale is a property of the scene: the value that separates a busy
    /// froxel from a quiet one in a hundred-light stress test washes
    /// every pixel red in a room with four lamps. Zero reads as
    /// [`LIGHTS_HOT_DEFAULT`] rather than dividing by nothing.
    pub debug_lights_hot: u32,
    /// How many of a froxel's punctual lights a pixel actually
    /// **evaluates**, chosen by estimated contribution (#826). 0 walks
    /// all of them, which is what every frame before this did.
    ///
    /// 🔴 Not [`Self::light_limit`] with a nicer name, and the
    /// difference is the whole issue. The limit truncates by the
    /// froxel's list ORDER, which is arbitrary — cross a cell boundary,
    /// the list reorders, and the set a pixel evaluates jumps. That is
    /// the froxel flicker the limit produced on the device. This picks
    /// by CONTRIBUTION and divides by the probability of having picked
    /// it, so the estimate is unbiased and the choice is continuous
    /// across the boundary the limit was discontinuous at.
    pub light_samples: u32,
    /// A counter that advances once per recorded frame, for anything
    /// that has to draw a *different* random number this frame than it
    /// drew last frame (#826).
    ///
    /// 🔴 Without this, [`Self::light_samples`] cannot ship. The sampler
    /// seeds on the froxel's grid index and the fragment's coordinate,
    /// both of which are the same every frame for a still camera — so
    /// the "noise" is a fixed pattern, and a temporal resolve averages a
    /// sequence of identical values to exactly that pattern. The engine
    /// is already paying 6.4 ms for that resolve; this word is what
    /// gives it something to average.
    ///
    /// Wraps, and is meant to: it only ever feeds a hash. It is the same
    /// counter `ContactShadowUbo::frame` takes, deliberately — two
    /// counters advancing independently would be two answers to "which
    /// frame is this".
    pub frame_index: u32,
    /// To 16. `debug_lights_hot` closed the previous group of four, so a
    /// fifth scalar opens a new one — see `IntiLight`'s three scalars
    /// for the same trap in the other direction.
    pub _pad_samples: [u32; 2],
}

/// Top of scale the lights-per-pixel view starts at.
///
/// Sixteen because a count the eye can quarter reads as a count. It is a
/// starting point and not a limit — the editor's control moves it, and
/// the whole reason it moves is that the right value is whatever makes
/// the picture stop being flat.
pub const LIGHTS_HOT_DEFAULT: u32 = 16;

/// Irradiance below which a light skips its specular layer (#821), as a
/// [`Resource`](kooch_core::resource::Resources).
///
/// `0.0` — the default — keeps every light on the full model, so a
/// project that never sets it renders exactly as before. It is a
/// resource rather than a constant because the useful value is a
/// property of the scene's exposure and light intensities, and the only
/// way to find it is to sweep it while watching the picture.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct SpecularFloor(pub f32);

impl Default for SpecularFloor {
    fn default() -> Self {
        Self(floor_from_environment())
    }
}

/// `KOOCH_SPECULAR_FLOOR=<lux>`, read once.
///
/// 🔴 An environment variable and not only an editor control, because
/// **the editor is not where this can be measured**. On a desktop GPU
/// the whole raster pass is 0.12 ms and switching every specular layer
/// off moves it by 0.001 — there is no bottleneck to remove. The frame
/// this exists for is a game on the OneXFly, launched over SSH, with no
/// editor in the process at all.
///
/// The same reasoning as `KOOCH_CLUSTERING`, learned the same way: a
/// knob that only exists in the editor is a knob that cannot be swept
/// on the machine whose numbers decide anything.
///
/// Unparseable keeps the default: a typo during a measurement run must
/// not silently change what is being measured.
fn floor_from_environment() -> f32 {
    static FLOOR: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *FLOOR.get_or_init(|| {
        let Ok(raw) = std::env::var("KOOCH_SPECULAR_FLOOR") else {
            return 0.0;
        };
        match raw.trim().parse::<f32>() {
            Ok(floor) if floor >= 0.0 => {
                tracing::info!(
                    "KOOCH_SPECULAR_FLOOR={floor}: lights under this irradiance shade \
                     diffuse-only"
                );
                floor
            }
            _ => {
                tracing::warn!("KOOCH_SPECULAR_FLOOR={raw:?} is not a number — keeping 0");
                0.0
            }
        }
    })
}

/// How many of a froxel's punctual lights a pixel evaluates, as a
/// [`Resource`](kooch_core::resource::Resources). `0` — the default —
/// evaluates all of them.
///
/// See [`IntiFrame::light_limit`] for what it is for. Like
/// `KOOCH_CLUSTERING` and `KOOCH_SPECULAR_FLOOR` it is an environment
/// variable, and for the third time for the same reason: the editor is
/// not where this can be measured. The desktop raster pass is 0.12 ms;
/// there is no bottleneck there to remove.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LightLimit(pub u32);

impl Default for LightLimit {
    fn default() -> Self {
        Self(limit_from_environment())
    }
}

/// `KOOCH_LIGHT_LIMIT=<n>`, read once.
///
/// Unparseable or negative keeps the default of "all": a typo during a
/// measurement run must not silently change what is being measured.
fn limit_from_environment() -> u32 {
    static LIMIT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        let Ok(raw) = std::env::var("KOOCH_LIGHT_LIMIT") else {
            return 0;
        };
        match raw.trim().parse::<u32>() {
            Ok(limit) => {
                tracing::info!(
                    "KOOCH_LIGHT_LIMIT={limit}: a pixel evaluates at most this many of its \
                     froxel's punctual lights. The picture is wrong on purpose — this \
                     measures whether the cost scales with that count."
                );
                limit
            }
            Err(_) => {
                tracing::warn!("KOOCH_LIGHT_LIMIT={raw:?} is not a count — keeping all lights");
                0
            }
        }
    })
}

/// How many of a froxel's punctual lights a pixel evaluates, picked by
/// estimated contribution (#826). 0 evaluates all of them.
///
/// 🔴 **Not [`LightLimit`] renamed.** The limit was an instrument: it
/// truncates the froxel's list by ORDER, keeps the first *n*, and drops
/// real light on purpose so a capture can answer whether the cost scales
/// with the count. It does — `shade = 11.11 ms + 1.06 ms per light` —
/// and it flickered on the device, because crossing a cell boundary
/// reorders the list and the kept set jumps.
///
/// This is the fix rather than the measurement. The lights are chosen in
/// proportion to what they contribute and each one's result is divided
/// by the probability of having chosen it, so the average over the
/// picked set estimates the sum over all of them. Two consequences that
/// the limit does not have: the estimate is **unbiased**, and the choice
/// is **continuous** across a cell boundary — the bright light near the
/// pixel stays the likely pick however the list is ordered.
///
/// It still costs something, and the something is noise rather than
/// darkness. Which is why it is a knob with a capture behind it and not
/// a default.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LightSamples(pub u32);

impl Default for LightSamples {
    fn default() -> Self {
        Self(light_samples_override().unwrap_or(0))
    }
}

/// `KOOCH_LIGHT_SAMPLES=<n>`, read once. Same shape as the three knobs
/// above, and for the fourth time the same reason: the editor is not
/// where this can be measured.
///
/// `None` when the variable says nothing, which is what lets the
/// project's `.rendersettings` value stand (#830). The variable outranks
/// the asset when it is set: it is the instrument, and an instrument
/// whose reading depends on which project happens to be open measures
/// nothing.
pub fn light_samples_override() -> Option<u32> {
    static SAMPLES: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *SAMPLES.get_or_init(|| {
        let Ok(raw) = std::env::var("KOOCH_LIGHT_SAMPLES") else {
            return None;
        };
        match raw.trim().parse::<u32>() {
            Ok(samples) => {
                // 🔴 The shader runs one stratum per thread and has 16
                // cells to serve out of 256, so it caps at
                // `MAX_TILE_STRATA`. Clamping here rather than there is
                // what makes a request past the cap say so: silently
                // producing the same picture for 8 and for 16 is a knob
                // that lies, and it would be found in a capture that
                // showed two identical measurements.
                let capped = samples.min(MAX_LIGHT_SAMPLES);
                if capped < samples {
                    tracing::warn!(
                        "KOOCH_LIGHT_SAMPLES={samples} is past the {MAX_LIGHT_SAMPLES} the \
                         tile can choose; using {capped}",
                    );
                }
                tracing::info!(
                    "KOOCH_LIGHT_SAMPLES={capped}: each froxel of a tile chooses this many \
                     of its punctual lights, in proportion to what they contribute, and \
                     every pixel of the froxel evaluates that choice weighted by the \
                     probability of the pick. Trades exactness for noise, not for darkness."
                );
                Some(capped)
            }
            Err(_) => {
                tracing::warn!(
                    "KOOCH_LIGHT_SAMPLES={raw:?} is not a count — leaving the project's own \
                     setting alone",
                );
                None
            }
        }
    })
}

/// Top of scale for `MeshletDebugMode::LightsPerPixel`, as a
/// [`Resource`](kooch_core::resource::Resources) the editor writes.
///
/// The same shape as [`DebugLight`]: a view's parameter belongs beside
/// the view, not threaded through the render stage.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LightsHot(pub u32);

impl Default for LightsHot {
    fn default() -> Self {
        Self(LIGHTS_HOT_DEFAULT)
    }
}

/// [`IntiFrame::debug_light`] when no light is isolated. Any index past
/// the light count reads the same way; this one is the deliberate value.
pub const NO_DEBUG_LIGHT: u32 = u32::MAX;

/// Which light `MeshletDebugMode::SingleLight` isolates (#743).
///
/// A [`Resource`](kooch_core::resource::Resources) the editor writes
/// from the World panel's selection, rather than a control of its own:
/// "one light at a time" is what selecting a light already means, and a
/// second list of lights to pick from is a second thing to keep in step
/// with the scene.
///
/// `None` — or an entity that is not an active light — renders magenta.
/// The two are the same answer to the viewer and neither is a failure.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct DebugLight(pub Option<kooch_ecs::entity::Entity>);

impl IntiFrame {
    pub fn new(
        ambient: &AmbientLight,
        exposure: &Exposure,
        camera_position: Vec3,
        light_count: u32,
    ) -> Self {
        Self {
            ambient_sky: ambient.sky_color.to_array(),
            light_count,
            ambient_ground: ambient.ground_color.to_array(),
            exposure: exposure.multiplier(),
            camera_position: camera_position.to_array(),
            ambient_intensity: ambient.intensity,
            camera_forward: Vec3::NEG_Z.to_array(),
            _pad_forward: 0.0,
            cascades: [GpuCascade::default(); FRAME_CASCADE_COUNT],
            spot_shadows: [GpuCascade::default(); MAX_SPOT_SHADOWS],
            spot_shadow_count: 0,
            point_shadow_count: 0,
            specular_floor: 0.0,
            light_limit: 0,
            point_shadows: [GpuPointShadow::default(); MAX_POINT_SHADOWS],
            shadows_enabled: 0,
            cascade_blend: 0.1,
            sun_softness: DEFAULT_SUN_SOFTNESS,
            debug_light: NO_DEBUG_LIGHT,
            view_z_row: [0.0, 0.0, -1.0, 0.0],
            cluster_dimensions: [0; 4],
            cluster_factors: [0.0; 4],
            cluster_capacity: 0,
            directional_count: 0,
            clustered: 0,
            debug_lights_hot: LIGHTS_HOT_DEFAULT,
            light_samples: 0,
            frame_index: 0,
            _pad_samples: [0; 2],
        }
    }

    /// How many of the buffer's leading entries are directional lights.
    pub fn with_directionals(mut self, count: u32) -> Self {
        self.directional_count = count;
        self
    }

    /// Points shading at the grid this view was clustered with (#780).
    ///
    /// Absent, `clustered` stays 0 and the shading loop walks every
    /// light — which is what it did before the grid existed, and what a
    /// path with no camera matrices still does.
    pub fn with_clusters(mut self, grid: &crate::ClusterGrid, view: Mat4, capacity: u32) -> Self {
        let dims = grid.dimensions;
        self.view_z_row = view.row(2).to_array();
        self.cluster_dimensions = [dims.x, dims.y, dims.z, grid.cluster_count()];
        self.cluster_factors = [
            grid.tile_factors.x,
            grid.tile_factors.y,
            grid.z_factors.x,
            grid.z_factors.y,
        ];
        self.cluster_capacity = capacity;
        self.clustered = 1;
        self
    }

    /// Isolates one light for the single-light debug view (#743).
    /// `None` — or an index the buffer does not hold — shows nothing.
    pub fn with_debug_light(mut self, index: Option<u32>) -> Self {
        self.debug_light = index.unwrap_or(NO_DEBUG_LIGHT);
        self
    }

    /// Sets the lights-per-pixel view's top of scale (#817).
    ///
    /// Clamped to at least one: a top of zero would divide the count by
    /// nothing and paint the whole screen the ramp's hot end, which is
    /// indistinguishable from the answer that means the grid is off.
    /// Sets the irradiance below which a light skips its specular
    /// layer (#821). Clamped at zero: a negative floor would mean
    /// nothing, and zero already means "never skip".
    pub fn with_specular_floor(mut self, floor: f32) -> Self {
        self.specular_floor = floor.max(0.0);
        self
    }

    /// Caps how many of a froxel's punctual lights a pixel evaluates
    /// (0 = all). See [`IntiFrame::light_limit`].
    pub fn with_light_limit(mut self, limit: u32) -> Self {
        self.light_limit = limit;
        self
    }

    /// How many of a froxel's punctual lights a pixel evaluates, picked
    /// by contribution (0 = all). See [`IntiFrame::light_samples`].
    pub fn with_light_samples(mut self, samples: u32) -> Self {
        self.light_samples = samples;
        self
    }

    /// The frame counter the samplers decorrelate on. See
    /// [`IntiFrame::frame_index`].
    pub fn with_frame_index(mut self, frame: u32) -> Self {
        self.frame_index = frame;
        self
    }

    pub fn with_lights_hot(mut self, hot: u32) -> Self {
        self.debug_lights_hot = hot.max(1);
        self
    }

    /// Attaches the shadows from [`FrameShadows`], if the frame has any.
    pub fn with_optional_shadows(self, shadows: Option<FrameShadows>) -> Self {
        match shadows {
            Some(s) => {
                let frame = if s.cascades_enabled {
                    self.with_shadows(s.camera_forward, s.cascades, s.blend, s.sun_softness)
                } else {
                    self
                };
                frame
                    .with_spot_shadows(s.spot_shadows, s.spot_shadow_count)
                    .with_point_shadows(s.point_shadows, s.point_shadow_count)
            }
            None => self,
        }
    }

    /// Attaches the shadow cascades and turns sampling on.
    pub fn with_shadows(
        mut self,
        camera_forward: Vec3,
        cascades: [GpuCascade; FRAME_CASCADE_COUNT],
        blend: f32,
        sun_softness: f32,
    ) -> Self {
        self.camera_forward = camera_forward.normalize_or(Vec3::NEG_Z).to_array();
        self.cascades = cascades;
        self.shadows_enabled = 1;
        self.cascade_blend = blend;
        self.sun_softness = sun_softness.max(0.0);
        self
    }

    /// Attaches the spot lights' shadow maps (#777).
    ///
    /// Separate from [`Self::with_shadows`], which turns
    /// `shadows_enabled` on: that flag gates the CASCADE sampling, and a
    /// scene can have a spot casting with no sun at all. A spot reads
    /// its own record and its own count, so it needs no flag.
    pub fn with_spot_shadows(
        mut self,
        spot_shadows: [GpuCascade; MAX_SPOT_SHADOWS],
        count: u32,
    ) -> Self {
        self.spot_shadows = spot_shadows;
        self.spot_shadow_count = count.min(MAX_SPOT_SHADOWS as u32);
        self
    }

    /// Attaches the point lights' cube maps (#778). Independent of the
    /// cascades for the same reason the spots' are.
    pub fn with_point_shadows(
        mut self,
        point_shadows: [GpuPointShadow; MAX_POINT_SHADOWS],
        count: u32,
    ) -> Self {
        self.point_shadows = point_shadows;
        self.point_shadow_count = count.min(MAX_POINT_SHADOWS as u32);
        self
    }
}

/// Everything the frame needs to sample shadows, as one value.
///
/// The producer is `kooch_render` — placing cascades needs the meshlet
/// pipeline's atlas, and this crate sits below it. Grouped rather than
/// passed as three parameters because they are only ever correct
/// together: cascades from one camera with the forward axis of another
/// puts every cascade boundary in the wrong place, and three loose
/// arguments is how that happens.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FrameShadows {
    /// Unit vector down the view axis, for the cascade selector.
    pub camera_forward: Vec3,
    pub cascades: [GpuCascade; FRAME_CASCADE_COUNT],
    /// Fraction of a split distance the cascades cross-fade over.
    pub blend: f32,
    /// Tangent of the sun's angular radius. See [`IntiFrame::sun_softness`].
    pub sun_softness: f32,
    /// 🔴 Whether the CASCADES are real.
    ///
    /// False when nothing directional casts and the frame exists only
    /// for spot lights (#777). The cascades still carry numbers — they
    /// were fitted to a stand-in direction so the pass has something
    /// coherent to not draw — and a directional light that does not cast
    /// would otherwise sample them and be shadowed by a sun that is not
    /// there.
    pub cascades_enabled: bool,
    /// One per shadow-casting spot light (#777).
    pub spot_shadows: [GpuCascade; MAX_SPOT_SHADOWS],
    /// How many of `spot_shadows` are live.
    pub spot_shadow_count: u32,
    /// One per shadow-casting point light (#778).
    pub point_shadows: [GpuPointShadow; MAX_POINT_SHADOWS],
    /// How many of `point_shadows` are live.
    pub point_shadow_count: u32,
    /// Which entity each live cube belongs to, in slot order.
    ///
    /// 🔴 Carried rather than recomputed. The slot a point light gets is
    /// its rank by distance to the camera, so the light buffer's walk
    /// order and the slot order are different orders — and the two
    /// places that need the mapping would have to sort identically, from
    /// the same camera, forever. One of them ranks; this array is the
    /// answer travelling to the other.
    pub point_entities: [kooch_ecs::entity::Entity; MAX_POINT_SHADOWS],
}

/// Tangent of the sun's angular radius, by default.
///
/// The real sun subtends about half a degree, so the honest value is
/// 0.0047 — and at that width PCSS is indistinguishable from PCF and
/// costs eight extra taps to prove it. 0.03 is roughly a three-degree
/// sun: about seven centimetres of penumbra per metre of gap, which is
/// what makes a shadow read as attached at its base and soft where it
/// is not. Every film and game widens it, for this reason.
pub const DEFAULT_SUN_SOFTNESS: f32 = 0.03;

#[cfg(test)]
mod tests;
