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
}

impl Default for WallSlide {
    fn default() -> Self {
        Self {
            max_fall: 2.0,
            grip: 0.3,
        }
    }
}

impl Component for WallSlide {}
