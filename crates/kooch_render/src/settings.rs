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

    /// Whether the sun casts shadows. Off frees the atlas entirely —
    /// 64 MiB at the default resolution.
    #[serde(default = "default_shadows_enabled")]
    #[reflect(group = "Sun shadows")]
    pub shadows_enabled: bool,
    /// How far from the camera shadows are drawn, in METRES.
    ///
    /// Raising this does not add shadows in the distance so much as move
    /// texels there: the four cascades are fitted to whatever range they
    /// are given, so a larger distance blurs the shadows near the
    /// camera, which are the ones being looked at.
    #[serde(default = "default_shadow_distance")]
    #[reflect(group = "Sun shadows")]
    pub shadow_distance: f32,
    /// Side of one shadow cascade in TEXELS. The atlas is twice this on
    /// each axis: 2048 costs 64 MiB, 1024 costs 16.
    #[serde(default = "default_cascade_texels")]
    #[reflect(group = "Sun shadows")]
    pub shadow_cascade_texels: u32,
    /// How soft shadow edges get with distance: the TANGENT of the sun's
    /// angular radius, so 0.03 widens a shadow by three centimetres per
    /// metre of gap between the object and what its shadow lands on.
    ///
    /// The honest value for our sun is 0.005, and at that width a soft
    /// shadow is indistinguishable from a hard one. Raise it for an
    /// overcast look; drop it to zero for a hard edge.
    #[serde(default = "default_sun_softness")]
    #[reflect(group = "Sun shadows")]
    pub sun_softness: f32,
    /// Where the first shadow cascade ends, in METRES. The other three
    /// follow logarithmically out to `shadow_distance`.
    ///
    /// **This is the one number that decides shadow sharpness near the
    /// camera.** Lower it and the near cascade covers less ground with
    /// the same texels; raise it and everything close gets coarser.
    /// Unity ships 10.05 and Godot 10.
    #[serde(default = "default_first_cascade")]
    #[reflect(group = "Sun shadows")]
    pub shadow_first_cascade_distance: f32,

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
    #[serde(default = "default_point_shadows")]
    #[reflect(group = "Sun shadows")]
    pub point_shadows: u32,

    /// Steps a contact-shadow ray takes. **Zero turns contact shadows
    /// off** for the whole project, whatever the individual lights say.
    ///
    /// Contact shadows are the few centimetres the cascades cannot
    /// resolve — where an object meets the floor. Cost is per light that
    /// opted in, per pixel it touches.
    #[serde(default = "default_contact_steps")]
    #[reflect(group = "Contact shadows")]
    pub contact_shadow_steps: u32,
    /// How far a contact-shadow ray travels, in METRES. Longer grounds
    /// objects that hover further from what they stand on, and costs the
    /// same — the steps just spread wider.
    #[serde(default = "default_contact_length")]
    #[reflect(group = "Contact shadows")]
    pub contact_shadow_length: f32,
    /// Thickness the march assumes every surface has, in METRES.
    ///
    /// The depth buffer records a surface, not a solid, so the march has
    /// to be told how deep to treat one. Too small and contact shadows
    /// detach from thin geometry; too large and a railing shadows
    /// everything behind it.
    #[serde(default = "default_contact_thickness")]
    #[reflect(group = "Contact shadows")]
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
    #[reflect(group = "Contact shadows")]
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
    /// 🔴 Legacy, and NOT shown in the inspector — the dropdown below
    /// replaced it, and two controls for one decision is how they end up
    /// disagreeing.
    ///
    /// ⚠️ The FIELD stays even though the control is gone, because it is
    /// what an old file's `upscale` is migrated from. Deleting it would
    /// take that with it and every project that had the resolve on would
    /// come back with it off — silently. It can go once no project in
    /// the wild predates `upscale`, which is not a date anyone can name.
    #[serde(default = "default_temporal_aa")]
    #[reflect(skip)]
    #[deprecated(note = "superseded by `upscale`; read only to migrate old files")]
    pub temporal_aa: bool,

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
}

/// The techniques the inspector offers.
///
/// 🎯 SGSR 2 is here now that its two passes are built. At a ratio of
/// 1:1 it resolves without upscaling, which is exactly the
/// configuration the transliteration is judged in: run it against the
/// engine's own resolve on the same frames and a port that is wrong
/// shows as a difference from a known-good image, not as a vague
/// softness. The resolution split is step 4 and is not built.
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
        label: "SGSR 2 — Qualcomm's, transliterated (1:1 for now)",
        value: 2,
    },
];

/// 🔴 A sentinel, not a technique: "this file predates the field".
///
/// It cannot be `0`, because `0` is a real answer — a project that
/// deliberately chose no resolve — and a file written before `upscale`
/// existed says nothing about which of the two it meant. The loader
/// resolves this from the legacy `temporal_aa` and writes a real value,
/// so nothing downstream ever sees it.
///
/// ⚠️ Without this the migration is silent data loss of exactly the
/// kind the project's rules single out: every project that had turned
/// the resolve ON would come back with it off, no error, no warning.
pub const UPSCALE_UNSET: u32 = u32::MAX;

fn default_upscale() -> u32 {
    UPSCALE_UNSET
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
/// It shipped with `compute_shading` and `temporal_aa` defaulting to
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
fn default_temporal_aa() -> bool {
    false
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
            aperture_f_stops: camera.aperture_f_stops,
            shutter_speed_s: camera.shutter_speed_s,
            sensitivity_iso: camera.sensitivity_iso,
            ambient_sky_color: ambient.sky_color,
            ambient_ground_color: ambient.ground_color,
            ambient_intensity: ambient.intensity,
            shadows_enabled: shadows.enabled,
            shadow_distance: shadows.max_distance,
            shadow_cascade_texels: shadows.cascade_texels,
            sun_softness: shadows.sun_softness,
            shadow_first_cascade_distance: shadows.first_cascade_distance,
            contact_shadow_steps: contact.linear_steps,
            contact_shadow_length: contact.length,
            contact_shadow_thickness: contact.thickness,
            contact_shadow_dominant: contact.dominant_only,
            point_shadows: shadows.point_shadows,
            compute_shading: default_compute_shading(),
            shading_rate: default_shading_rate(),
            temporal_aa: default_temporal_aa(),
            // A value, not the sentinel: nothing to migrate from.
            upscale: 0,
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

    pub fn shadows(&self) -> ShadowSettings {
        ShadowSettings {
            max_distance: self.shadow_distance,
            cascade_texels: self.shadow_cascade_texels,
            enabled: self.shadows_enabled,
            sun_softness: self.sun_softness,
            first_cascade_distance: self.shadow_first_cascade_distance,
            point_shadows: crate::shadow::point_shadows_from_environment()
                .unwrap_or(self.point_shadows),
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
        crate::quality::TemporalSettings::new(technique)
    }

    /// The technique this file asks for, with the legacy field
    /// resolved.
    ///
    /// A file written before `upscale` existed carries the sentinel, and
    /// what it meant lives in `temporal_aa`. Reading it here rather than
    /// only in the loader means a `RenderSettings` built by hand — a
    /// test, a game that sets it in code — migrates too.
    pub fn technique(&self) -> crate::quality::UpscaleTechnique {
        if self.upscale == UPSCALE_UNSET {
            #[allow(deprecated)]
            return if self.temporal_aa {
                crate::quality::UpscaleTechnique::Taa
            } else {
                crate::quality::UpscaleTechnique::None
            };
        }
        crate::quality::UpscaleTechnique::from_asset(self.upscale)
    }

    /// Replaces the sentinel with the technique it stood for, so the
    /// inspector never has to draw a value that is not on its menu and
    /// the next save writes the real one.
    pub fn migrate_upscale(&mut self) {
        if self.upscale == UPSCALE_UNSET {
            self.upscale = match self.technique() {
                crate::quality::UpscaleTechnique::None => 0,
                crate::quality::UpscaleTechnique::Taa => 1,
                crate::quality::UpscaleTechnique::Sgsr2 => 2,
            };
        }
    }

    /// Publishes into the `Resources` the shading model already reads.
    ///
    /// The indirection is the point: `inti_pbr.wgsl` and `GpuLights`
    /// never learn what an asset is, so a game that sets `Exposure`
    /// directly keeps working and a headless test needs no file.
    pub fn apply(&self, resources: &mut Resources) {
        resources.insert(Exposure::from_physical(self.camera()));
        resources.insert(self.ambient());
        resources.insert(self.shadows());
        resources.insert(self.contact_shadows());
        let shading = self.shading();
        resources.insert(shading);
        resources.insert(self.temporal());
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
        let mut settings: RenderSettings =
            ron::from_str(text).map_err(|e| AssetError::Loader(Box::new(e)))?;
        // The one migration step: a file older than `upscale` carries
        // the sentinel, and what it meant is in `temporal_aa`.
        settings.migrate_upscale();
        Ok(settings)
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
    let temporal = settings.temporal();
    let stale = resources.get::<Exposure>() != Some(&exposure)
        || resources.get::<AmbientLight>() != Some(&ambient)
        || resources.get::<ShadowSettings>() != Some(&shadows)
        || resources.get::<ContactShadowSettings>() != Some(&contact)
        || resources.get::<crate::quality::ShadingSettings>() != Some(&shading)
        || resources.get::<crate::quality::TemporalSettings>() != Some(&temporal);
    if stale {
        settings.apply(resources);
        tracing::debug!(
            target: "kooch_render::settings",
            ev100 = exposure.ev100,
            "render settings applied",
        );
    }
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
