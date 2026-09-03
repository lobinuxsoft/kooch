//! [`WallSlide`] — falling slowly down something you cannot climb.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Caps how fast a character falls while it is pressed against a wall.
///
/// Reads [`Touching`](crate::Touching); casts nothing of its own. Add it
/// beside a [`WallJump`](crate::WallJump) and a wall becomes somewhere
/// to stop and think rather than somewhere to fall past.
///
/// # Why a cap and not a friction
///
/// A friction would make the slide speed depend on how far the
/// character had already fallen, so arriving from a great height would
/// scrape down at a completely different rate than stepping on from the
/// side. The cap is the same every time, which is what makes it
/// something a player can rely on.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct WallSlide {
    /// Fastest it may fall while on a wall, in m/s.
    pub max_fall: f32,
    /// How hard it must be steered into the wall to hold on, from `0`
    /// (clings to anything it brushes) to `1` (straight at it).
    ///
    /// Without this a character sails past every wall at slide speed,
    /// including the ones it is only running alongside.
    pub grip: f32,
    /// How hard it is held against the wall while gripping, in m/s².
    ///
    /// Arriving at speed, the solver pushes the capsule back out of the
    /// wall — and with the air push deliberately not aimed into it,
    /// nothing brings it back. The character bounces off and drifts
    /// away mid-slide.
    ///
    /// This is that push, made explicit and authorable, rather than the
    /// contact friction it used to get by accident. Speed *away* from
    /// the wall is dropped outright while gripping: a bounce is the
    /// wall's answer to arriving, not something the character asked
    /// for.
    pub stick: f32,
}

impl Default for WallSlide {
    fn default() -> Self {
        Self {
            max_fall: 2.0,
            grip: 0.3,
            stick: 12.0,
        }
    }
}

impl Component for WallSlide {}
