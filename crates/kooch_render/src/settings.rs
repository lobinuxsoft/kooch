//! `RenderSettings` — what the **author** decided the project looks like (#744).

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
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(category = "Rendering")]
pub struct RenderSettings {
    /// Aperture, as an f-stop. **Lower is brighter**: f/1.4 gathers four times the light of f/2.8.
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
    #[serde(default = "default_sky")]
    #[reflect(group = "Ambient light")]
    pub ambient_sky_color: glam::Vec3,
    /// Ambient light arriving from world down, as linear RGB. Bounce
    /// off the ground, not sky.
    #[serde(default = "default_ground")]
    #[reflect(group = "Ambient light")]
    pub ambient_ground_color: glam::Vec3,
    /// Ambient illuminance in LUX, on the same scale as a directional light. An office is 320; a
    /// directional light defaults to 10 000.
    #[serde(default = "default_ambient_intensity")]
    #[reflect(group = "Ambient light")]
    pub ambient_intensity: f32,

    /// Whether the sun's shadow comes from the VIRTUAL page pool instead of the four cascades
    /// (#866).
    #[serde(default = "default_virtual_shadows")]
    #[reflect(group = "Shadows: virtual pages")]
    pub virtual_shadows: bool,
    /// Shadow texels per screen pixel, as a PERCENTAGE.
    #[serde(default = "default_shadow_density")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_DENSITY_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_density: u32,
    /// Physical pages the pool holds, which IS the memory budget: one page is 64 KiB at 128 texels
    /// and `Depth32Float`.
    #[serde(default = "default_shadow_pool_pages")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_POOL_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_pool_pages: u32,
    /// Width of the page shadows' PCF footprint, in shadow texels.
    #[serde(default = "default_shadow_softness")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_SOFTNESS_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_softness: u32,
    /// Projected radius, in screen pixels, under which a local light becomes DISTANT: it casts from
    /// ONE page per cube face instead of a six-face mip chain (#1009).
    #[serde(default = "default_shadow_min_pixels")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_MIN_PIXELS_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_min_pixels: u32,
    /// How far a shadow lookup steps along the receiver's NORMAL before comparing, as a MULTIPLE OF
    /// THE CLIPMAP TEXEL it landed on.
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
    /// A ceiling on the world-space normal step, in METRES. 0 disables it, which is what shipped.
    #[serde(default = "default_shadow_bias_max")]
    #[reflect(
        group = "Shadows: virtual pages",
        range = SHADOW_BIAS_MAX_RANGE,
        shown_when = PAGES_ON
    )]
    pub shadow_bias_max: f32,
    /// A ceiling on the receiver's own depth GRADIENT, as a slope — `tan` of the incidence, per
    /// axis.
    #[serde(default = "default_shadow_bias_slope")]
    #[reflect(
        group = "Shadows: virtual pages",
        range = SHADOW_BIAS_SLOPE_RANGE,
        shown_when = PAGES_ON
    )]
    pub shadow_bias_slope: f32,
    /// March the shadow atlas instead of sampling one texel through a PCF box.
    #[serde(default)]
    #[reflect(group = "Shadows: virtual pages", shown_when = PAGES_ON)]
    pub shadow_page_march: bool,
    /// Runs the expansion from the GEOMETRY: one thread per surviving meshlet, descending the page
    /// pyramid to the pages it lands in, instead of pairing every listed page against every
    /// survivor (#1022).
    #[serde(default = "default_shadow_page_geometry")]
    #[reflect(group = "Shadows: virtual pages", shown_when = PAGES_ON)]
    pub shadow_page_geometry: bool,
    /// How far, in PAGES, a receiver dilates its page request — Epic's `PageDilationOffset`. 0
    /// turns it off.
    #[serde(default = "default_shadow_page_halo")]
    #[reflect(group = "Shadows: virtual pages", shown_when = PAGES_ON)]
    pub shadow_page_halo: f32,
    /// How much simplification error a meshlet may show before the cull picks a finer level, in
    /// PIXELS.
    #[serde(default = "default_meshlet_lod_error")]
    #[reflect(group = "Geometry", range = MESHLET_LOD_ERROR_RANGE)]
    pub meshlet_lod_error: f32,
    /// Projected radius, in pixels, under which an instance is dropped before it becomes meshlets
    /// (#1002). `0` = draw everything the frustum holds, which is what shipped.
    #[serde(default = "default_meshlet_min_pixels")]
    #[reflect(group = "Geometry", range = MESHLET_MIN_PIXELS_RANGE)]
    pub meshlet_min_pixels: f32,
    /// Reject instances first and expand only the survivors (#1002),
    /// instead of dispatching `instances × the heaviest mesh`.
    #[serde(default = "default_meshlet_two_level")]
    #[reflect(group = "Geometry")]
    pub meshlet_two_level: bool,
    /// How far a local light may cast shadow pages from, in multiples of its OWN range. The light
    /// still SHADES past it; only its shadow stops being paid for.
    #[serde(default = "default_shadow_light_reach")]
    #[reflect(
        group = "Shadows: virtual pages",
        choices = SHADOW_LIGHT_REACH_CHOICES,
        shown_when = PAGES_ON
    )]
    pub shadow_light_reach: u32,
    /// Whether shadows are drawn at all. Off frees the atlas entirely — 64 MiB at the default
    /// resolution — and the cube maps with it.
    #[serde(default = "default_shadows_enabled")]
    #[reflect(group = "Shadows")]
    pub shadows_enabled: bool,
    /// How far from the camera shadows are drawn, in METRES.
    #[serde(default = "default_shadow_distance")]
    #[reflect(group = "Shadows: sun cascades", shown_when = PAGES_OFF)]
    pub shadow_distance: f32,
    /// How soft shadow edges get with distance: the TANGENT of the sun's angular radius, so 0.03
    /// widens a shadow by three centimetres per metre of gap between the object and what its shadow
    /// lands on.
    #[serde(default = "default_sun_softness")]
    #[reflect(group = "Shadows: sun cascades", shown_when = PAGES_OFF)]
    pub sun_softness: f32,
    /// Where the first shadow cascade ends, in METRES. The other three follow logarithmically out
    /// to `shadow_distance`.
    #[serde(default = "default_first_cascade")]
    #[reflect(group = "Shadows: sun cascades", shown_when = PAGES_OFF)]
    pub shadow_first_cascade_distance: f32,

    /// Side of one atlas layer in TEXELS. The atlas is twice this on each axis: 2048 costs 64 MiB,
    /// 1024 costs 16.
    #[serde(default = "default_cascade_texels")]
    #[reflect(group = "Shadows: atlas", shown_when = PAGES_OFF)]
    pub shadow_cascade_texels: u32,

    /// How many point lights may cast a cube map at once (#849).
    #[serde(default = "default_point_shadows")]
    #[reflect(group = "Shadows: atlas", shown_when = PAGES_OFF)]
    pub point_shadows: u32,

    /// Steps a contact-shadow ray takes. **Zero turns contact shadows off** for the whole project,
    /// whatever the individual lights say.
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
    #[serde(default = "default_contact_thickness")]
    #[reflect(group = "Shadows: contact")]
    pub contact_shadow_thickness: f32,
    /// March once per pixel — for the light that lit it hardest — instead of once for every light
    /// that reaches it (#845).
    #[serde(default = "default_contact_dominant")]
    #[reflect(group = "Shadows: contact")]
    pub contact_shadow_dominant: bool,

    /// Shading as a COMPUTE pass over the visibility buffer (#824) rather than a fragment one.
    #[serde(default = "default_compute_shading")]
    #[reflect(group = "Shading")]
    pub compute_shading: bool,
    /// Pixels per shaded sample, per AXIS (#825). 1 shades every pixel; 2 shades one per 2x2 quad
    /// and reconstructs the rest using the visibility buffer as the edge guide.
    #[serde(default = "default_shading_rate")]
    #[reflect(group = "Shading", choices = SHADING_RATE_CHOICES)]
    pub shading_rate: u32,

    /// Temporal anti-aliasing (#481): each frame samples a different sub-pixel position and is
    /// blended with the ones before it.

    /// Which temporal technique resolves the frame (#481, #536).
    #[serde(default = "default_upscale")]
    #[reflect(group = "Temporal", choices = UPSCALE_CHOICES)]
    pub upscale: u32,

    /// How much smaller than the window the scene is RENDERED, as a percentage of the output's
    /// width (#481, step 4).
    #[serde(default = "default_render_scale")]
    #[reflect(
        group = "Temporal",
        choices = RENDER_SCALE_CHOICES,
        shown_when = UPSCALES_WHEN
    )]
    pub render_scale: u32,

    /// How hard the finished image is sharpened, 0..=100 (#481, step 5).
    #[serde(default = "default_sharpening")]
    #[reflect(group = "Temporal")]
    pub sharpening: u32,

    /// Samples the filter takes along the long axis of a texture footprint, 1..=16.
    #[serde(default = "default_anisotropy")]
    #[reflect(group = "Texture filtering", choices = ANISOTROPY_CHOICES)]
    pub anisotropy: u32,

    /// Whether the surface waits for the vblank before presenting.
    #[serde(default = "default_vsync")]
    #[reflect(group = "Presentation")]
    pub vsync: bool,

    /// Where the window sits between a rectangle on a desktop and the whole screen: 0 windowed, 1
    /// borderless, 2 fullscreen, 3 exclusive.
    #[serde(default = "default_window_mode")]
    #[reflect(group = "Presentation", choices = WINDOW_MODE_CHOICES)]
    pub window_mode: u32,
}

impl Default for RenderSettings {
    /// What a NEW settings asset holds, and what the engine uses with no asset at all.
    fn default() -> Self {
        Self {
            shadow_normal_bias: default_shadow_normal_bias(),
            shadow_depth_bias: default_shadow_depth_bias(),
            shadow_bias_max: default_shadow_bias_max(),
            shadow_bias_slope: default_shadow_bias_slope(),
            shadow_page_march: false,
            shadow_page_geometry: default_shadow_page_geometry(),
            shadow_page_halo: default_shadow_page_halo(),
            meshlet_lod_error: default_meshlet_lod_error(),
            meshlet_min_pixels: default_meshlet_min_pixels(),
            meshlet_two_level: default_meshlet_two_level(),
            aperture_f_stops: default_aperture(),
            shutter_speed_s: default_shutter(),
            sensitivity_iso: default_iso(),
            ambient_sky_color: default_sky(),
            ambient_ground_color: default_ground(),
            ambient_intensity: default_ambient_intensity(),
            shadows_enabled: default_shadows_enabled(),
            shadow_distance: default_shadow_distance(),
            shadow_cascade_texels: default_cascade_texels(),
            shadow_softness: default_shadow_softness(),
            shadow_min_pixels: default_shadow_min_pixels(),
            shadow_light_reach: default_shadow_light_reach(),
            sun_softness: default_sun_softness(),
            shadow_first_cascade_distance: default_first_cascade(),
            contact_shadow_steps: default_contact_steps(),
            contact_shadow_length: default_contact_length(),
            contact_shadow_thickness: default_contact_thickness(),
            contact_shadow_dominant: default_contact_dominant(),
            point_shadows: default_point_shadows(),
            compute_shading: default_compute_shading(),
            shading_rate: default_shading_rate(),
            upscale: default_upscale(),
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
    pub fn meshlet_lod(&self) -> crate::meshlet::MeshletLodSettings {
        crate::meshlet::MeshletLodSettings {
            // Clamped rather than trusted: the range constrains the Inspector, and a settings file
            // is a text file anyone can write a zero into. A target of zero means no level is ever
            // fine enough and the cull emits nothing at all.
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
            page_halo: self.shadow_page_halo,
            max_distance: self.shadow_distance,
            cascade_texels: self.shadow_cascade_texels,
            enabled: self.shadows_enabled,
            sun_softness: self.sun_softness,
            first_cascade_distance: self.shadow_first_cascade_distance,
            point_shadows: crate::shadow::point_shadows_from_environment()
                .unwrap_or(self.point_shadows),
            // 🔴 `KOOCH_PAGE_MARKING=1` is a FORCE on top of the asset, the way
            // `point_shadows_from_environment` is — and it is applied HERE rather than at the call
            // site.
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
    /// applied on top — see `crate::contact_shadow::steps_from_environment`
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

    /// 🔴 Gated on the shading path, not merely documented as needing it.
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
    pub fn window_mode(&self) -> kooch_core::window_mode::WindowMode {
        kooch_core::window_mode::WindowMode::from_asset(self.window_mode)
    }

    /// The technique this file asks for.
    pub fn technique(&self) -> crate::quality::UpscaleTechnique {
        crate::quality::UpscaleTechnique::from_asset(self.upscale)
    }

    /// Publishes into the `Resources` the shading model already reads.
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

mod apply;
mod choices;

pub use apply::apply_render_settings_system;
pub use choices::shadow_density_choices;
use choices::*;

#[cfg(test)]
mod tests;
