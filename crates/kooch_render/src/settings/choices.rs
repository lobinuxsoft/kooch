//! What each setting offers the Inspector — choices, ranges, conditions — and what an old file's missing field reads as.

use super::*;

/// 🔴 Serialised into user projects, so append-only.
pub(super) const WINDOW_MODE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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
pub(super) const ANISOTROPY_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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
pub(super) fn default_vsync() -> bool {
    false
}

/// Windowed. The mode that works on every platform and every
/// compositor, and the one an author has to opt out of rather than into
/// — a file that predates this field must not take the display.
pub(super) fn default_window_mode() -> u32 {
    1
}

/// Off, like every other quality setting in this file: it costs
/// bandwidth on the surfaces that already cover the most pixels, and a
/// project that never asked for that should not pay it.
pub(super) fn default_anisotropy() -> u32 {
    2
}

/// The techniques the inspector offers.
pub(super) const UPSCALE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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
pub(super) fn default_upscale() -> u32 {
    2
}

/// 🔴 `render_scale` is only shown for techniques that reconstruct.
pub(super) static UPSCALES_WHEN: kooch_ecs::reflect::FieldCondition =
    kooch_ecs::reflect::FieldCondition {
        field: "upscale",
        // Sgsr2, Fsr3, Dlss.
        values: &[2, 3, 4],
    };

/// AMD's preset ladder, by the name each ratio is known under, because
/// "Quality" is what a player recognises and 67 % is what it means.
pub(super) const RENDER_SCALE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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

pub(super) fn default_render_scale() -> u32 {
    50
}

/// One shadow texel per screen pixel, which is Epic's ask and the ceiling of the list.
pub(super) fn default_shadow_softness() -> u32 {
    3
}

pub(super) fn default_shadow_density() -> u32 {
    50
}

/// 8: on a 1080p screen, a light whose whole reach projects to a
/// 16-pixel blob. Its shadow would be a page nobody can see.
pub(super) fn default_shadow_min_pixels() -> u32 {
    8
}

/// One pixel, which is what `MeshletLodSettings::default` has always been. Changing the default
/// would change every existing project's geometry on the frame this landed. What `inti_pbr.wgsl`
/// held as a constant before the settings could reach it.
pub(super) fn default_shadow_normal_bias() -> f32 {
    4.0
}

/// Likewise.
pub(super) fn default_shadow_depth_bias() -> f32 {
    0.2
}

/// 🔴 Zero — OFF — because turning it on is a behaviour change and the value that is right has not
/// been measured yet. The same reasoning as `shadow_light_reach`: a cap chosen from arithmetic
/// rather than from a picture is a number nobody validated.
pub(super) fn default_shadow_bias_max() -> f32 {
    0.5
}

/// 🔴 NOT zero, unlike `default_shadow_bias_max`.
pub(super) fn default_shadow_bias_slope() -> f32 {
    0.0
}

/// Half a page each way, so a receiver in the outer half of its page already asks for the
/// neighbour. Epic size their dilation from a border in texels; half a page is the coarsest version
/// of the same idea and the one whose cost is easiest to read off the panel.
pub(super) fn default_shadow_page_halo() -> f32 {
    0.0
}

pub(super) fn default_meshlet_lod_error() -> f32 {
    0.5
}

/// 🔴 Zero — every instance the frustum holds is drawn, which is what shipped. Turning it on hides
/// geometry, and what a non-zero value hides is a judgement the author makes, not a default anyone
/// inherits.
pub(super) fn default_meshlet_min_pixels() -> f32 {
    0.0
}

/// On. The two-level shape draws exactly the same meshlets as the
/// rectangle did — it only stops dispatching the ones that were never
/// going to survive a bounds check.
pub(super) fn default_meshlet_two_level() -> bool {
    true
}

/// 🔴 Zero, because turning it on is a behaviour change and nothing has
/// measured what it costs. A threshold chosen from a whiteboard is how
/// `DEFAULT_PAGES` came to sit at half of Epic's for a year.
pub(super) fn default_shadow_light_reach() -> u32 {
    2
}

/// 🔴 Off. The cascades are what every scene in the project was authored against, and a technique
/// that replaces them cannot become the default on the frame it first renders.
pub(super) fn default_virtual_shadows() -> bool {
    true
}

pub(super) fn default_shadow_pool_pages() -> u32 {
    kooch_render_pool_default()
}

/// Indirection so the default and the pool agree without this module
/// reaching into the shadow tree for a constant it would then have to
/// keep in step by hand.
pub(super) fn kooch_render_pool_default() -> u32 {
    crate::shadow::pages::pool::DEFAULT_PAGES
}

/// Powers of two, because the atlas is a square grid of pages and the
/// labels are the only place the megabytes are ever stated.
pub(super) const SHADOW_POOL_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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
pub(super) const PAGES_ON: kooch_ecs::reflect::FieldCondition =
    kooch_ecs::reflect::FieldCondition {
        field: "virtual_shadows",
        values: &[1],
    };

/// The other side of [`PAGES_ON`].
pub(super) const PAGES_OFF: kooch_ecs::reflect::FieldCondition =
    kooch_ecs::reflect::FieldCondition {
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
pub(super) const SHADOW_DENSITY_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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
pub(super) const MESHLET_MIN_PIXELS_RANGE: kooch_ecs::reflect::FieldRange =
    kooch_ecs::reflect::FieldRange {
        min: 0.0,
        max: 64.0,
        step: 0.5,
    };

pub(super) const SHADOW_NORMAL_BIAS_RANGE: kooch_ecs::reflect::FieldRange =
    kooch_ecs::reflect::FieldRange {
        min: 0.0,
        max: 8.0,
        step: 0.05,
    };

pub(super) const SHADOW_DEPTH_BIAS_RANGE: kooch_ecs::reflect::FieldRange =
    kooch_ecs::reflect::FieldRange {
        min: 0.0,
        max: 0.5,
        step: 0.001,
    };

/// Up to a metre. Past that it is not a cap on anything the chain
/// produces below level 14.
pub(super) const SHADOW_BIAS_MAX_RANGE: kooch_ecs::reflect::FieldRange =
    kooch_ecs::reflect::FieldRange {
        min: 0.0,
        max: 1.0,
        step: 0.005,
    };

/// 0 is one depth for every tap — what shipped. 8 is `tan 83°`, past
/// which the clamp is not clamping anything the geometry reaches.
pub(super) const SHADOW_BIAS_SLOPE_RANGE: kooch_ecs::reflect::FieldRange =
    kooch_ecs::reflect::FieldRange {
        min: 0.0,
        max: 8.0,
        step: 0.1,
    };

pub(super) const MESHLET_LOD_ERROR_RANGE: kooch_ecs::reflect::FieldRange =
    kooch_ecs::reflect::FieldRange {
        min: 0.01,
        max: 8.0,
        step: 0.01,
    };

pub(super) const SHADOW_LIGHT_REACH_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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

pub(super) const SHADOW_MIN_PIXELS_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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

pub(super) const SHADOW_SOFTNESS_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
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
pub(super) fn default_sharpening() -> u32 {
    50
}

/// The two rates that exist. Quarter rate is deliberately absent: at 4x4 the upsample's guide stops
/// being able to reconstruct a silhouette, which is a different technique rather than a bigger
/// constant.
pub(super) const SHADING_RATE_CHOICES: &[kooch_ecs::reflect::FieldChoice] = &[
    kooch_ecs::reflect::FieldChoice {
        label: "Full — one sample per pixel",
        value: 1,
    },
    kooch_ecs::reflect::FieldChoice {
        label: "Half — one sample per 2x2 quad",
        value: 2,
    },
];

pub(super) fn default_aperture() -> f32 {
    PhysicalCamera::default().aperture_f_stops
}
pub(super) fn default_shutter() -> f32 {
    PhysicalCamera::default().shutter_speed_s
}
pub(super) fn default_iso() -> f32 {
    PhysicalCamera::default().sensitivity_iso
}
pub(super) fn default_sky() -> glam::Vec3 {
    glam::Vec3::new(0.27272725, 0.58441556, 1.0)
}
pub(super) fn default_ground() -> glam::Vec3 {
    AmbientLight::default().ground_color
}
pub(super) fn default_ambient_intensity() -> f32 {
    AmbientLight::default().intensity
}
pub(super) fn default_shadows_enabled() -> bool {
    ShadowSettings::default().enabled
}
pub(super) fn default_shadow_distance() -> f32 {
    200.0
}
pub(super) fn default_cascade_texels() -> u32 {
    512
}
pub(super) fn default_sun_softness() -> f32 {
    0.005
}
pub(super) fn default_first_cascade() -> f32 {
    20.0
}
pub(super) fn default_contact_steps() -> u32 {
    6
}
pub(super) fn default_contact_length() -> f32 {
    ContactShadowSettings::default().length
}
pub(super) fn default_point_shadows() -> u32 {
    100
}
pub(super) fn default_contact_dominant() -> bool {
    false
}
pub(super) fn default_contact_thickness() -> f32 {
    ContactShadowSettings::default().thickness
}
/// 🔴 These four are the ENGINE's defaults, deliberately, and an earlier version of this file got it
/// wrong.
pub(super) fn default_compute_shading() -> bool {
    true
}
pub(super) fn default_shading_rate() -> u32 {
    crate::meshlet::ShadingRate::Full.factor()
}

/// Whether the expansion runs from the geometry (#1022). Measured at
/// ~1850x under the pairing it replaces, on `dense.scene`.
pub(super) fn default_shadow_page_geometry() -> bool {
    true
}
