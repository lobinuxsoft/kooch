//! Point light component.
//!
//! Omnidirectional light source that emits from a single point in all
//! directions. Uses [`Transform`](crate::Transform)`::position` as origin;
//! rotation is ignored.

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Omnidirectional point light source.
///
/// # Default
///
/// - `active`: true
/// - `color`: white `(1, 1, 1)`
/// - `intensity`: [`lumens::ROOM_LIGHT_NO_GI`](crate::light_consts::lumens::ROOM_LIGHT_NO_GI)
/// - `range`: 10.0
/// - `radius`: 0.0 (a point, as every light in the engine was before)
/// - `cast_shadows`: true
/// - `contact_shadows`: false
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct PointLight {
    /// Whether this light contributes to the scene.
    ///
    /// Unchecking it is not the same as deleting the entity: the light
    /// keeps its position, colour and gizmo, and costs nothing.
    pub active: bool,
    /// Colour, as linear RGB.
    ///
    /// Linear, not sRGB — this is the colour the light physically emits,
    /// not a colour picked on a screen. Doubling a channel doubles the
    /// energy in it.
    pub color: Vec3,
    /// Luminous flux, in LUMENS — the total light emitted in every
    /// direction combined.
    ///
    /// Not the same unit as a DirectionalLight's intensity, which is in
    /// lux. A real 9 W LED bulb is 800 lm; a car headlight is 20 000 lm.
    ///
    /// ⚠️ The default is far higher than any real bulb, on purpose. The
    /// renderer computes direct light only, so the bounces that make a
    /// real room bright are missing and an honest 800 lm reads as almost
    /// nothing. See `light_consts::lumens` for named values.
    pub intensity: f32,
    /// Distance at which the light reaches exactly zero, in world units.
    ///
    /// Not a physical property — real light never stops — but a budget:
    /// nothing beyond this range pays for this light. The editor's wire
    /// sphere draws precisely this boundary, and scales with the
    /// entity's transform the way the shading does.
    pub range: f32,
    /// Radius of the emitting sphere, in world units. `0` is a
    /// mathematical point.
    ///
    /// A real lamp has a size, and its size is what decides how big the
    /// highlight it leaves on a glossy surface is. Growing this widens
    /// that highlight without making the surface brighter.
    ///
    /// ⚠️ **Specular only.** It does not soften the diffuse falloff and
    /// it does not soften shadows — a soft shadow is a separate
    /// technique driven by the same number (#477), not a consequence of
    /// this one. Bevy's `PointLight::radius` is documented the same way.
    pub radius: f32,
    /// Whether this light casts shadows.
    ///
    /// Since #778 this is a cube map and a promise the engine keeps —
    /// for **at most `MAX_POINT_SHADOWS` lights at a time**. Past that a
    /// light keeps lighting the scene and stops casting; which ones lose
    /// their cube is decided per frame by how much a cube spent on them
    /// would show, so the Inspector cannot answer it for a given light.
    pub cast_shadows: bool,
    /// Whether this light marches the depth buffer for contact shadows.
    ///
    /// Off by default, unlike [`DirectionalLight`](crate::DirectionalLight):
    /// the march costs per light per pixel, and a scene has one sun but
    /// can have fifty lamps. Turn it on for the few whose contact with
    /// the floor the viewer actually looks at.
    ///
    /// 🔴 **Measured, and it is the most expensive thing a point light
    /// can be asked to do.** In `many_lights.scene` — a hundred lamps,
    /// eight steps each — turning this off across the scene took
    /// `shade: compute` from 24.860 ms to 11.081 on the OneXFly, against
    /// a whole-frame budget of 13.9. Unlike the cube map it has **no
    /// cap**: every light that reaches a pixel marches for it.
    pub contact_shadows: bool,
}

impl Default for PointLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: crate::light_consts::lumens::ROOM_LIGHT_NO_GI,
            range: 10.0,
            radius: 0.0,
            cast_shadows: true,
            contact_shadows: false,
        }
    }
}

impl Component for PointLight {}

#[cfg(test)]
mod tests;
