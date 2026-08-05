//! Spot light component.
//!
//! Cone-shaped light source. Uses [`Transform`](crate::Transform)`::position`
//! as origin and the forward vector of `rotation` as the cone axis. Intensity
//! falls off between `inner_angle` (full) and `outer_angle` (zero).

use glam::Vec3;

use crate::component::Component;

#[allow(unused_imports)]
use crate::Reflect;

/// Cone-shaped spot light source.
///
/// # Default
///
/// - `active`: true
/// - `color`: white `(1, 1, 1)`
/// - `intensity`: 800.0 lumens
/// - `range`: 10.0
/// - `inner_angle`: 30.0 degrees
/// - `outer_angle`: 45.0 degrees
/// - `cast_shadows`: true
#[derive(Debug, Clone, Copy, Reflect)]
#[reflect(category = "Rendering")]
pub struct SpotLight {
    /// Whether this light contributes to the scene.
    ///
    /// Unchecking it is not the same as deleting the entity: the light
    /// keeps its position, colour and gizmo, and costs nothing.
    pub active: bool,
    /// Colour, as linear RGB.
    ///
    /// Linear, not sRGB — this is the colour the light physically emits,
    /// not a colour picked on a screen.
    pub color: Vec3,
    /// Luminous flux, in LUMENS — as if the light emitted in every
    /// direction, not only into its cone.
    ///
    /// Narrowing the cone therefore aims the light without brightening
    /// it. Unity and Bevy both chose this; the alternative is an artist
    /// widening a cone and watching the scene go dark.
    ///
    /// ⚠️ The default is far higher than any real fixture, on purpose —
    /// the renderer computes direct light only. See
    /// `light_consts::lumens`.
    pub intensity: f32,
    /// Distance at which the light reaches exactly zero, in world units.
    ///
    /// A budget rather than a physical property — real light never
    /// stops. The editor's wire cone is drawn to exactly this length.
    pub range: f32,
    /// HALF-angle of the fully-lit cone, in degrees, measured from the
    /// axis to the edge.
    ///
    /// Half-angle, like Unreal — not Unity's single full `spotAngle`. A
    /// 30 here is a 60-degree cone. Everything inside is at full
    /// intensity; between this and `outer_angle` is the penumbra.
    pub inner_angle: f32,
    /// HALF-angle at which the light reaches zero, in degrees.
    ///
    /// The gap between this and `inner_angle` is the soft edge. Equal
    /// values give a hard-edged cone.
    pub outer_angle: f32,
    /// Whether this light casts shadows.
    ///
    /// ⚠️ Not implemented yet — shadows land with #476 / #477. The field
    /// is stored and saved; today nothing reads it.
    pub cast_shadows: bool,
}

impl Default for SpotLight {
    fn default() -> Self {
        Self {
            active: true,
            color: Vec3::ONE,
            intensity: crate::light_consts::lumens::ROOM_LIGHT_NO_GI,
            range: 10.0,
            inner_angle: 30.0,
            outer_angle: 45.0,
            cast_shadows: true,
        }
    }
}

impl Component for SpotLight {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::Reflect;

    #[test]
    fn default_values() {
        let l = SpotLight::default();
        assert!(l.active);
        assert_eq!(l.color, Vec3::ONE);
        // Deliberately far above a real fixture: direct lighting only,
        // so the bounces that make a real room bright are missing. Goes
        // back to a real bulb the day #450 lands.
        assert_eq!(l.intensity, crate::light_consts::lumens::ROOM_LIGHT_NO_GI);
        assert_eq!(l.range, 10.0);
        assert_eq!(l.inner_angle, 30.0);
        assert_eq!(l.outer_angle, 45.0);
        assert!(l.cast_shadows);
    }

    #[test]
    fn reflect_fields() {
        let l = SpotLight::default();
        let fields = l.reflect_fields();
        let names: Vec<&str> = fields.iter().map(|f| f.name).collect();
        assert_eq!(
            names,
            &[
                "active",
                "color",
                "intensity",
                "range",
                "inner_angle",
                "outer_angle",
                "cast_shadows",
            ]
        );
    }
}
