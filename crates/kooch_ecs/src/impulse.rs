//! What shakes the world, and the event that carries it (#1419).
//!
//! 🔴 Here rather than in the camera, because an impulse is a fact about the WORLD. A camera is one
//! listener; a sound, a dust puff and a controller rumble are others, and none of them should have
//! to learn what a camera is to hear a landing. `kooch_camera` listens, `kooch_character` emits,
//! and neither depends on the other.

pub mod shape;

use glam::Vec3;

use crate::Reflect;
use crate::component::Component;
use crate::reflect::FieldRange;

/// What an entity emits when something happens to it.
///
/// Authored on the thing that shakes the world — what lands, fires or explodes — and turned into an
/// [`Impulse`] by whatever notices the event.
///
/// ⚠️ **The alias is load-bearing.** This lived at `kooch_camera::impulse::ImpulseSource`, and a
/// scene resolves a component by its type NAME: without it the component would vanish from every
/// entity that carries one, silently, on the first load after the move.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Impulse", alias = "kooch_camera::impulse::ImpulseSource")]
pub struct ImpulseSource {
    /// Which signal: one of [`shape`]'s constants.
    #[reflect(choices = shape::SHAPE_CHOICES)]
    pub shape: u32,
    /// How far a listener is pushed, per axis, in metres at the source.
    pub amplitude: Vec3,
    /// How long the whole signal lasts.
    #[reflect(range = DURATION_RANGE)]
    pub duration: f32,
    /// Inside this, full strength. Cinemachine's `ImpactRadius`.
    #[reflect(range = DISTANCE_RANGE)]
    pub radius: f32,
    /// How much further it takes to fade to nothing past the radius. Cinemachine's
    /// `DissipationDistance`.
    #[reflect(range = DISTANCE_RANGE)]
    pub dissipation: f32,
    /// Which listeners hear it. One sharing no bit is not shaken.
    #[reflect(layers)]
    pub channels: u32,
}

const DURATION_RANGE: FieldRange = FieldRange {
    min: 0.01,
    max: 10.0,
    step: 0.01,
};

const DISTANCE_RANGE: FieldRange = FieldRange {
    min: 0.0,
    max: 1000.0,
    step: 0.5,
};

impl Default for ImpulseSource {
    fn default() -> Self {
        Self {
            shape: shape::BUMP,
            // A knock you notice without it reading as a bug the first time it fires.
            amplitude: Vec3::new(0.0, 0.3, 0.0),
            duration: 0.2,
            radius: 5.0,
            dissipation: 20.0,
            channels: u32::MAX,
        }
    }
}

impl Component for ImpulseSource {}

/// One impulse, fired. Published as an event so anything can hear it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Impulse {
    /// Where it happened, in world space.
    pub at: Vec3,
    pub amplitude: Vec3,
    pub shape: u32,
    pub duration: f32,
    pub radius: f32,
    pub dissipation: f32,
    pub channels: u32,
}

impl Impulse {
    /// `source`'s signal, fired from `at` and scaled by `strength` — how hard the thing that fired
    /// it was hit, from `0` to `1`.
    pub fn from_source(source: ImpulseSource, at: Vec3, strength: f32) -> Self {
        Self {
            at,
            amplitude: source.amplitude * strength.clamp(0.0, 1.0),
            shape: source.shape,
            duration: source.duration,
            radius: source.radius.max(0.0),
            dissipation: source.dissipation.max(0.0),
            channels: source.channels,
        }
    }
}
