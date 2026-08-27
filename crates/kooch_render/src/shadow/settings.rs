//! What the author decides about shadows.
//!
//! A `Resource` with a default, published from `.rendersettings` the
//! same way exposure and ambient are (#744). Separate from
//! `RenderSettings` as a type because the shadow pass reads it and the
//! shading model does not — and because a `Resource` nobody can author
//! is the failure this engine has now committed three times.

/// How far from the camera shadows are drawn, in metres.
///
/// Two hundred is a scene, not a planet. The cascades are fitted to
/// whatever range they are given, so raising this does not add shadows
/// in the distance — it moves the near cascade's texels outward and
/// blurs the shadows that are actually being looked at.
pub const DEFAULT_SHADOW_DISTANCE: f32 = 100.0;

/// Where the first cascade ends, in metres.
///
/// The split scheme is logarithmic from here, so this is the single
/// number that decides how much resolution the shadows near the camera
/// get. Unity ships 10.05, Godot 10, and Bevy takes both as its
/// reference — ten metres around the camera is what a scene at human
/// scale wants, and anchoring at the camera's near plane instead spends
/// the first cascade on the first few centimetres.
pub const DEFAULT_FIRST_CASCADE_DISTANCE: f32 = 10.0;

/// Side of one cascade in texels, when the author has not said.
///
/// Mirrors [`crate::shadow::DEFAULT_CASCADE_SIZE`]; stated here so the
/// settings type has a complete default without the caller reaching for
/// the atlas.
pub const DEFAULT_CASCADE_TEXELS: u32 = super::atlas::DEFAULT_CASCADE_SIZE;

/// Shadow settings, as a `Resource`.
/// Cubes a project gets before it asks for more.
///
/// Four, unchanged from when it was a hard constant: it is what every
/// capture so far was taken against, and a default that quietly costs a
/// project 192 MiB of VRAM would be a worse surprise than a shadow that
/// pops.
pub const DEFAULT_POINT_SHADOWS: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowSettings {
    /// Distance from the camera the cascades cover, in metres.
    pub max_distance: f32,
    /// Side of one cascade in texels. The atlas is twice this on each
    /// axis, so 2048 costs 64 MiB and 1024 costs 16.
    pub cascade_texels: u32,
    /// Whether the pass runs at all. Off means no atlas is allocated —
    /// the sixty-four megabytes are not spent by a project that does not
    /// want shadows.
    pub enabled: bool,
    /// Tangent of the sun's angular radius: how much wider a shadow gets
    /// per metre between the blocker and the surface it lands on.
    pub sun_softness: f32,
    /// Where the first cascade ends, in metres. The rest follow
    /// logarithmically out to `max_distance`.
    pub first_cascade_distance: f32,
    /// How many point lights may hold a cube at once (#849).
    ///
    /// 🔴 The one number that decides whether shadows **pop**. Which
    /// lights hold the cubes is chosen per frame from where the camera
    /// is, so when the budget is smaller than the number of lights on
    /// screen, moving hands the cubes to different lamps and a shadow
    /// appears or vanishes with no authored reason. Raising it does not
    /// make shadows better — it makes them stop changing.
    ///
    /// **6 MiB each**, six faces of 512² at `Depth32Float`. Clamped to
    /// [`MAX_POINT_SHADOWS`](kooch_lighting::MAX_POINT_SHADOWS), which
    /// sizes the uniform array and costs nothing unspent.
    pub point_shadows: u32,
    /// Whether the sun's shadow comes from the virtual page pool
    /// instead of the four cascades (#866/#477).
    ///
    /// 🔴 Here, in the **published** settings, and not read off
    /// `RenderSettings` at the call site. That is not a style
    /// preference: `RenderSettings` is never inserted as a `Resources`
    /// value — `apply` publishes these derived structs instead — so a
    /// frame that asked for the whole struct got `None` in every build
    /// and silently took its fallback. That is precisely how this
    /// feature shipped inert, and the profile that caught it showed two
    /// captures, on and off, byte for byte the same.
    pub virtual_pages: bool,
    /// Shadow texels per screen pixel, as a percentage.
    pub page_density: u32,
    /// Physical pages the pool holds, which is the memory budget.
    pub pool_pages: u32,
    /// PCF footprint width of the page readers, in shadow texels.
    /// 1 = bilinear; wider = Castano-style box with bilinear edges,
    /// `(width + 1)²` loads per light per pixel (#941).
    pub page_softness: u32,
    /// How far a shadow lookup steps along the receiver's NORMAL before
    /// comparing, as a multiple of the clipmap texel it landed on.
    ///
    /// 🔴 The multiplier is per TEXEL, and a clipmap texel is 0.1 mm at
    /// level 0 and five metres at level 16. So this number is not a
    /// distance — it decides one, and the distance it decides spans
    /// five orders of magnitude across the chain. See
    /// [`Self::page_bias_max`], which is the cap that keeps it finite.
    pub page_normal_bias: f32,
    /// How far the same lookup steps TOWARDS the light, in metres.
    /// Constant across the chain, unlike the normal step.
    pub page_depth_bias: f32,
    /// A ceiling on the world-space normal step, in metres. 0 = none,
    /// which is what shipped.
    ///
    /// 🔴 Without it the step grows with the texel: 0.58 m at clipmap
    /// level 12, 9.2 m at level 16. A receiver pushed metres along its
    /// own normal leaves the volume its caster shadows, the depth test
    /// answers LIT, and the shadow ends in a straight line at the level
    /// boundary — with the page present, resident and correctly drawn.
    pub page_bias_max: f32,
    /// Projected radius in screen pixels under which a local light
    /// casts no pages (#944). 0 = every light casts.
    pub page_min_pixels: u32,
    /// How far a local light may cast pages from, in multiples of its
    /// OWN range. 0 = no distance limit, which is what shipped.
    ///
    /// 🔴 Not the same question as [`Self::page_min_pixels`], though
    /// both turn a light away. That one is a projected SIZE, so the
    /// distance it implies scales with the light's range and with the
    /// viewport — at 808x439 a range-50 light does not fall under eight
    /// pixels until 2.4 km, and a threshold high enough to cut it at a
    /// hundred metres also cuts a small light beside the camera. This
    /// one scales with the light instead of with the screen.
    pub page_light_reach: u32,
}

impl ShadowSettings {
    /// The budget, never past what the uniform can address.
    pub fn point_budget(&self) -> usize {
        (self.point_shadows as usize).min(kooch_lighting::MAX_POINT_SHADOWS)
    }
}

impl Default for ShadowSettings {
    fn default() -> Self {
        Self {
            max_distance: DEFAULT_SHADOW_DISTANCE,
            cascade_texels: DEFAULT_CASCADE_TEXELS,
            enabled: true,
            sun_softness: kooch_lighting::DEFAULT_SUN_SOFTNESS,
            first_cascade_distance: DEFAULT_FIRST_CASCADE_DISTANCE,
            point_shadows: DEFAULT_POINT_SHADOWS,
            // 🔴 Off, and the environment variable is applied where the
            // asset is read rather than here: a `Default` that consulted
            // the environment would make every test depend on the shell
            // it ran in.
            virtual_pages: false,
            page_density: 100,
            pool_pages: crate::shadow::pages::pool::DEFAULT_PAGES,
            page_softness: 1,
            page_normal_bias: 1.8,
            page_depth_bias: 0.02,
            page_bias_max: 0.0,
            page_min_pixels: 8,
            // 🔴 Off, because it is a behaviour change and nothing has
            // measured what it costs yet: a light out of reach stops
            // casting, and a threshold picked from a whiteboard is how
            // `DEFAULT_PAGES` ended up at half of Epic's.
            page_light_reach: 0,
        }
    }
}

impl ShadowSettings {
    /// Cascade size clamped to something a device will allocate.
    ///
    /// The atlas is twice this per axis, and 8192 is the smallest
    /// `max_texture_dimension_2d` any target guarantees — so a cascade
    /// larger than 4096 is a texture creation failure, which surfaces as
    /// a panic in wgpu rather than as a bad-looking shadow.
    pub fn clamped_texels(&self) -> u32 {
        self.cascade_texels.clamp(256, 4096)
    }
}

#[cfg(test)]
mod tests;

/// `KOOCH_POINT_SHADOWS=<count>`, read once (#849).
///
/// The seventh variable of its family, for the reason all of them exist:
/// the question this answers — does raising the budget stop the shadows
/// from popping, and what does it cost — is answered on the OneXFly
/// through Steam, where reaching the settings asset means a repack and a
/// copy.
pub fn point_shadows_from_environment() -> Option<u32> {
    static COUNT: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
    *COUNT.get_or_init(|| {
        let count = std::env::var("KOOCH_POINT_SHADOWS")
            .ok()
            .and_then(|raw| raw.trim().parse::<u32>().ok());
        if let Some(count) = count {
            tracing::info!(
                target: "kooch_render::shadow",
                "KOOCH_POINT_SHADOWS={count}: up to {count} point lights hold a cube \
                 at once, {} MiB of it",
                count as u64 * 6,
            );
        }
        count
    })
}

#[cfg(test)]
mod budget_tests;
