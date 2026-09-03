//! [`WallRun`] — carrying speed along a wall instead of down it.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Running along a wall for as long as the speed and the clock allow.
///
/// The other answer to a wall, and a different move from
/// [`WallSlide`](crate::WallSlide) rather than a setting of it:
///
/// | | [`WallSlide`](crate::WallSlide) | `WallRun` |
/// |---|---|---|
/// | approached | head on | at a angle, with speed |
/// | what it does | falls slowly | holds gravity off |
/// | ends | when you let go | when the clock runs out |
///
/// Both on one character is one asking to fall and one asking not to,
/// so a scene authors whichever it means.
///
/// # Why a clock and not a fuel bar
///
/// A run that ends on a timer ends at a place the player can see coming
/// — they watched it start. One that ends when some quantity runs out
/// ends wherever the arithmetic happened to land, and reads as the wall
/// letting go at random.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct WallRun {
    /// Least speed **along** the wall that will start a run, in m/s.
    ///
    /// What makes this a run rather than a cling: arriving slowly, or
    /// straight at the wall, is not a wall run in any game that has one.
    pub entry: f32,
    /// How long one run lasts, in seconds.
    ///
    /// Spent on the wall, and only refilled by standing on something —
    /// otherwise a character chains the same wall for ever by letting go
    /// of it for a frame.
    pub duration: f32,
    /// How much of gravity is held off while running, from `0` to `1`.
    ///
    /// Not `1`: a run that does not sag has no clock a player can feel,
    /// and the sag is what tells them it is ending before it does.
    pub hold: f32,
    /// Push toward the wall while running, in m/s².
    ///
    /// The solver pushes the capsule back out of what it hits, and the
    /// air push is deliberately not aimed into walls, so without this a
    /// run drifts off the wall it is on.
    pub stick: f32,
    /// How far the body banks towards the wall, from `0` (upright) to
    /// `1` (lying against it).
    ///
    /// The whole read of a wall run from outside. Upright, a character
    /// running along a wall looks like one hovering beside it.
    pub bank: f32,
}

impl Default for WallRun {
    fn default() -> Self {
        Self {
            entry: 3.0,
            duration: 1.6,
            hold: 0.85,
            stick: 12.0,
            bank: 0.55,
        }
    }
}

impl Component for WallRun {}
