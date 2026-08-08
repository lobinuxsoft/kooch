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
    pub aperture_f_stops: f32,
    /// Shutter time in SECONDS. Longer is brighter. 1/125 is 0.008.
    #[serde(default = "default_shutter")]
    pub shutter_speed_s: f32,
    /// Film speed. Higher is brighter — and in a real camera noisier,
    /// though here it is brightness only.
    #[serde(default = "default_iso")]
    pub sensitivity_iso: f32,

    /// Ambient light arriving from world up, as linear RGB.
    ///
    /// Stands in for a sky the renderer cannot sample yet. Without it a
    /// metal facing away from every light renders pure black — correct
    /// for the model, and indistinguishable from a bug.
    #[serde(default = "default_sky")]
    pub ambient_sky_color: glam::Vec3,
    /// Ambient light arriving from world down, as linear RGB. Bounce
    /// off the ground, not sky.
    #[serde(default = "default_ground")]
    pub ambient_ground_color: glam::Vec3,
    /// Ambient illuminance in LUX, on the same scale as a directional
    /// light. An office is 320; a directional light defaults to 10 000.
    ///
    /// Raise it and shadowed surfaces lift; raise it far and the scene
    /// flattens, because ambient arrives from everywhere and therefore
    /// describes no direction.
    #[serde(default = "default_ambient_intensity")]
    pub ambient_intensity: f32,

    /// Whether the sun casts shadows. Off frees the atlas entirely —
    /// 64 MiB at the default resolution.
    #[serde(default = "default_shadows_enabled")]
    pub shadows_enabled: bool,
    /// How far from the camera shadows are drawn, in METRES.
    ///
    /// Raising this does not add shadows in the distance so much as move
    /// texels there: the four cascades are fitted to whatever range they
    /// are given, so a larger distance blurs the shadows near the
    /// camera, which are the ones being looked at.
    #[serde(default = "default_shadow_distance")]
    pub shadow_distance: f32,
    /// Side of one shadow cascade in TEXELS. The atlas is twice this on
    /// each axis: 2048 costs 64 MiB, 1024 costs 16.
    #[serde(default = "default_cascade_texels")]
    pub shadow_cascade_texels: u32,
    /// How soft shadow edges get with distance: the TANGENT of the sun's
    /// angular radius, so 0.03 widens a shadow by three centimetres per
    /// metre of gap between the object and what its shadow lands on.
    ///
    /// The honest value for our sun is 0.005, and at that width a soft
    /// shadow is indistinguishable from a hard one. Raise it for an
    /// overcast look; drop it to zero for a hard edge.
    #[serde(default = "default_sun_softness")]
    pub sun_softness: f32,
    /// Where the first shadow cascade ends, in METRES. The other three
    /// follow logarithmically out to `shadow_distance`.
    ///
    /// **This is the one number that decides shadow sharpness near the
    /// camera.** Lower it and the near cascade covers less ground with
    /// the same texels; raise it and everything close gets coarser.
    /// Unity ships 10.05 and Godot 10.
    #[serde(default = "default_first_cascade")]
    pub shadow_first_cascade_distance: f32,

    /// Steps a contact-shadow ray takes. **Zero turns contact shadows
    /// off** for the whole project, whatever the individual lights say.
    ///
    /// Contact shadows are the few centimetres the cascades cannot
    /// resolve — where an object meets the floor. Cost is per light that
    /// opted in, per pixel it touches.
    #[serde(default = "default_contact_steps")]
    pub contact_shadow_steps: u32,
    /// How far a contact-shadow ray travels, in METRES. Longer grounds
    /// objects that hover further from what they stand on, and costs the
    /// same — the steps just spread wider.
    #[serde(default = "default_contact_length")]
    pub contact_shadow_length: f32,
    /// Thickness the march assumes every surface has, in METRES.
    ///
    /// The depth buffer records a surface, not a solid, so the march has
    /// to be told how deep to treat one. Too small and contact shadows
    /// detach from thin geometry; too large and a railing shadows
    /// everything behind it.
    #[serde(default = "default_contact_thickness")]
    pub contact_shadow_thickness: f32,
}

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
fn default_contact_thickness() -> f32 {
    ContactShadowSettings::default().thickness
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
        }
    }

    pub fn contact_shadows(&self) -> ContactShadowSettings {
        ContactShadowSettings {
            linear_steps: self.contact_shadow_steps,
            length: self.contact_shadow_length,
            thickness: self.contact_shadow_thickness,
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
    let stale = resources.get::<Exposure>() != Some(&exposure)
        || resources.get::<AmbientLight>() != Some(&ambient)
        || resources.get::<ShadowSettings>() != Some(&shadows)
        || resources.get::<ContactShadowSettings>() != Some(&contact);
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
