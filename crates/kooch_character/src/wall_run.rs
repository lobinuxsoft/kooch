//! [`WallRun`] — carrying speed along a wall instead of down it.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Running along a wall while speed and a clock allow — a different move from
/// [`WallSlide`](crate::WallSlide), which falls slowly when approached head on. A clock ends where
/// the player saw it coming.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct WallRun {
    /// Least speed **along** the wall that starts a run, in m/s — a slow or head-on arrival is not
    /// one.
    pub entry: f32,
    /// How long one run lasts, in seconds, refilled only by standing, or letting go for a frame
    /// would chain it forever.
    pub duration: f32,
    /// How much gravity is held off while running, `0` to `1` — below `1`, the sag tells a player
    /// the run is ending.
    pub hold: f32,
    /// Push towards the wall while running, in m/s², or the solver's push-back drifts the run off
    /// the wall.
    pub stick: f32,
    /// How far the body banks towards the wall, `0` upright to `1` lying against it — upright reads
    /// as hovering.
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
