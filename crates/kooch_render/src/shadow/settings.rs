//! What the author decides about shadows.

/// How far from the camera shadows are drawn, in metres.
pub const DEFAULT_SHADOW_DISTANCE: f32 = 100.0;

/// Where the first cascade ends, in metres.
pub const DEFAULT_FIRST_CASCADE_DISTANCE: f32 = 10.0;

/// Side of one cascade in texels, when the author has not said.
pub const DEFAULT_CASCADE_TEXELS: u32 = super::atlas::DEFAULT_CASCADE_SIZE;

/// Shadow settings, as a `Resource`. Cubes a project gets before it asks for more.
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
    pub point_shadows: u32,
    /// Whether the sun's shadow comes from the virtual page pool instead of the four cascades
    /// (#866/#477).
    pub virtual_pages: bool,
    /// Shadow texels per screen pixel, as a percentage.
    pub page_density: u32,
    /// Physical pages the pool holds, which is the memory budget.
    pub pool_pages: u32,
    /// PCF footprint width of the page readers, in shadow texels.
    /// 1 = bilinear; wider = Castano-style box with bilinear edges,
    /// `(width + 1)²` loads per light per pixel (#941).
    pub page_softness: u32,
    /// How far a shadow lookup steps along the receiver's NORMAL before comparing, as a multiple of
    /// the clipmap texel it landed on.
    pub page_normal_bias: f32,
    /// How far the same lookup steps TOWARDS the light, in metres.
    /// Constant across the chain, unlike the normal step.
    pub page_depth_bias: f32,
    /// A ceiling on the world-space normal step, in metres. 0 = none, which is what shipped.
    pub page_bias_max: f32,
    /// A ceiling on the receiver's own depth GRADIENT, as a slope (`tan` of the incidence, per
    /// axis) — #1017.
    pub page_bias_slope: f32,
    /// Whether the shading MARCHES the atlas rather than sampling one texel through a PCF box
    /// (#1017).
    pub page_march: bool,
    /// Whether the expansion runs from the geometry rather than pairing
    /// pages against survivors (#1022). See
    /// `RenderSettings::shadow_page_geometry`.
    pub page_geometry: bool,
    /// How far, in pages, a receiver dilates its request (#1022). See
    /// `RenderSettings::shadow_page_halo`.
    pub page_halo: f32,
    /// Projected radius in screen pixels under which a local light becomes DISTANT: one page per
    /// cube face rather than a chain (#1009). 0 = every light gets a chain. See
    /// `RenderSettings::shadow_min_pixels`.
    pub page_min_pixels: u32,
    /// How far a local light may cast pages from, in multiples of its OWN range. 0 = no distance
    /// limit, which is what shipped.
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
            // 🔴 Off, and the environment variable is applied where the asset is read rather than
            // here: a `Default` that consulted the environment would make every test depend on the
            // shell it ran in.
            virtual_pages: false,
            page_density: 100,
            pool_pages: crate::shadow::pages::pool::DEFAULT_PAGES,
            page_softness: 1,
            page_normal_bias: 1.8,
            page_depth_bias: 0.02,
            page_bias_max: 0.0,
            // 🔴 ON, unlike the cap above it.
            page_bias_slope: 4.0,
            // 🔴 OFF. It replaces the reader every shipped frame goes through and it costs rays
            // times steps of lookups against the box's taps.
            page_march: false,
            // Off: the shape is new and what it costs on a real scene
            // is a measurement nobody has taken. The pairs are the same
            // pairs either way, so nothing is lost by measuring first.
            page_geometry: false,
            page_halo: 0.5,
            page_min_pixels: 8,
            // 🔴 Off, because it is a behaviour change and nothing has measured what it costs yet: a
            // light out of reach stops casting, and a threshold picked from a whiteboard is how
            // `DEFAULT_PAGES` ended up at half of Epic's.
            page_light_reach: 0,
        }
    }
}

impl ShadowSettings {
    /// Cascade size clamped to something a device will allocate.
    pub fn clamped_texels(&self) -> u32 {
        self.cascade_texels.clamp(256, 4096)
    }
}

#[cfg(test)]
mod tests;

/// `KOOCH_POINT_SHADOWS=<count>`, read once (#849).
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
