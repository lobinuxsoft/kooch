//! [`WallSlide`] — falling slowly down something you cannot climb.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Caps a character's fall while pressed against a wall, reading [`Touching`](crate::Touching). A
/// cap, not friction, so the slide is the same from any height.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct WallSlide {
    /// Fastest it may fall while on a wall, in m/s.
    pub max_fall: f32,
    /// How hard it must steer into the wall to hold on, `0` to `1` — or it clings to walls it only
    /// runs alongside.
    pub grip: f32,
    /// How hard it is held against the wall while gripping, in m/s², so arriving at speed does not
    /// bounce it off; speed away from the wall is dropped.
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
