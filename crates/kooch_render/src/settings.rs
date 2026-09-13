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
    false
}

/// Windowed. The mode that works on every platform and every
/// compositor, and the one an author has to opt out of rather than into
/// — a file that predates this field must not take the display.
fn default_window_mode() -> u32 {
    1
}

/// Off, like every other quality setting in this file: it costs
/// bandwidth on the surfaces that already cover the most pixels, and a
/// project that never asked for that should not pay it.
fn default_anisotropy() -> u32 {
    2
}

/// The techniques the inspector offers.
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
    2
}

/// 🔴 `render_scale` is only shown for techniques that reconstruct.
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
    50
}

/// One shadow texel per screen pixel, which is Epic's ask and the ceiling of the list.
fn default_shadow_softness() -> u32 {
    3
}

fn default_shadow_density() -> u32 {
    50
}

/// 8: on a 1080p screen, a light whose whole reach projects to a
/// 16-pixel blob. Its shadow would be a page nobody can see.
fn default_shadow_min_pixels() -> u32 {
    8
}

/// One pixel, which is what `MeshletLodSettings::default` has always been. Changing the default
/// would change every existing project's geometry on the frame this landed. What `inti_pbr.wgsl`
/// held as a constant before the settings could reach it.
fn default_shadow_normal_bias() -> f32 {
    4.0
}

/// Likewise.
fn default_shadow_depth_bias() -> f32 {
    0.2
}

/// 🔴 Zero — OFF — because turning it on is a behaviour change and the value that is right has not
/// been measured yet. The same reasoning as `shadow_light_reach`: a cap chosen from arithmetic
/// rather than from a picture is a number nobody validated.
fn default_shadow_bias_max() -> f32 {
    0.5
}

/// 🔴 NOT zero, unlike `default_shadow_bias_max`.
fn default_shadow_bias_slope() -> f32 {
    0.0
}

/// Half a page each way, so a receiver in the outer half of its page already asks for the
/// neighbour. Epic size their dilation from a border in texels; half a page is the coarsest version
/// of the same idea and the one whose cost is easiest to read off the panel.
fn default_shadow_page_halo() -> f32 {
    0.0
}

fn default_meshlet_lod_error() -> f32 {
    0.5
}

/// 🔴 Zero — every instance the frustum holds is drawn, which is what shipped. Turning it on hides
/// geometry, and what a non-zero value hides is a judgement the author makes, not a default anyone
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
    2
}

/// 🔴 Off. The cascades are what every scene in the project was authored against, and a technique
/// that replaces them cannot become the default on the frame it first renders.
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

/// 🔴 A control that silently does nothing is worse than an absent one: it invites the reading that
/// the setting was tried and did not help.
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

/// 🔴 100 % is a REFERENCE, not a maximum, and the list used to stop there — "nobody reaches for a
/// graphics option hoping to find something above maximum".
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

/// The footprint widths on offer. `(width + 1)²` is the loads per light per pixel, which is why the
/// list is short and the wide end is named after its bill. The distance gate, in multiples of a
/// light's own range.
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
        label: "Off — only the derived test demotes",
        value: 0,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "4 px — demote only the invisible",
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
        label: "32 px — distant lamps drop to one page",
        value: 32,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "64 px — only lamps filling the screen keep a chain",
        value: 64,
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

/// No sharpening, for the reason no resolve is the default: this rewrites every pixel of a finished
/// image, and a project that never mentioned it did not ask for that.
fn default_sharpening() -> u32 {
    50
}

/// The two rates that exist. Quarter rate is deliberately absent: at 4x4 the upsample's guide stops
/// being able to reconstruct a silhouette, which is a different technique rather than a bigger
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
    glam::Vec3::new(0.27272725, 0.58441556, 1.0)
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
    200.0
}
fn default_cascade_texels() -> u32 {
    512
}
fn default_sun_softness() -> f32 {
    0.005
}
fn default_first_cascade() -> f32 {
    20.0
}
fn default_contact_steps() -> u32 {
    6
}
fn default_contact_length() -> f32 {
    ContactShadowSettings::default().length
}
fn default_point_shadows() -> u32 {
    100
}
fn default_contact_dominant() -> bool {
    false
}
fn default_contact_thickness() -> f32 {
    ContactShadowSettings::default().thickness
}
/// 🔴 These four are the ENGINE's defaults, deliberately, and an earlier version of this file got it
/// wrong.
fn default_compute_shading() -> bool {
    true
}
fn default_shading_rate() -> u32 {
    crate::meshlet::ShadingRate::Full.factor()
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

/// Whether the expansion runs from the geometry (#1022). Measured at
/// ~1850x under the pairing it replaces, on `dense.scene`.
fn default_shadow_page_geometry() -> bool {
    true
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

#[cfg(test)]
mod tests;

/// Finds the project's settings asset, loads it, and publishes the values the shading model reads.
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
        // 🔴 After `apply`, which inserts the technique the FILE asked for. A project authored on a
        // machine with DLSS is opened on one without, and the value it wrote is still the right
        // thing to keep in the asset — what must not survive is the engine then trying to run it.
        resources.insert(temporal);
        tracing::debug!(
            target: "kooch_render::settings",
            ev100 = exposure.ev100,
            "render settings applied",
        );
    }
}

/// Downgrades DLSS to the engine's own resolve when this build, or this adapter, cannot run it
/// (#536).
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
