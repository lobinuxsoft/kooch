//! What shakes the world, and the event that carries it (#1419, #1421).
//!
//! 🔴 Here rather than in the camera, because an impulse is a fact about the WORLD. A camera is one
//! listener; a sound, a dust puff and a controller rumble are others, and none of them should have
//! to learn what a camera is to hear a landing. `kooch_camera` listens, `kooch_character` emits,
//! and neither depends on the other.

pub mod shape;

use glam::Vec3;

use crate::Reflect;
use crate::reflect::FieldRange;

/// The signal a trigger fires: what it feels like, and how far it carries.
///
/// 🔴 Not a component. It was one — `ImpulseSource` — on the theory that a signal and its trigger
/// are separate statements, which is #1397's rule about a vcam's body and aim. **That rule is about
/// alternatives that exclude each other.** A trigger and its signal are not alternatives: they are
/// always authored together, so splitting them stated nothing and spread one setting across two
/// components. Each trigger carries one of these inline (#1421).
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
pub struct ImpulseSignal {
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

impl Default for ImpulseSignal {
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
    /// `signal`, fired from `at` and scaled by `strength` — how hard the thing that fired it was
    /// hit, from `0` to `1`.
    pub fn new(signal: ImpulseSignal, at: Vec3, strength: f32) -> Self {
        Self {
            at,
            amplitude: signal.amplitude * strength.clamp(0.0, 1.0),
            shape: signal.shape,
            duration: signal.duration,
            radius: signal.radius.max(0.0),
            dissipation: signal.dissipation.max(0.0),
            channels: signal.channels,
        }
    }
}
