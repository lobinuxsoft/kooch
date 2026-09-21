//! [`Sprint`] — running, as a multiplier on walking.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Running: [`Walk`](crate::Walk) with its numbers scaled while running — a modifier on an existing
/// term. Held, it runs while `wanted`; toggled, a press starts it and stopping ends it.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct Sprint {
    /// Written by gameplay every frame, like
    /// [`Facing`](crate::Facing): held is running, released is not.
    pub wanted: bool,
    /// A press starts running and another ends it, as does letting go of the move — held otherwise.
    pub toggle: bool,
    /// What the top speed is multiplied by.
    pub speed: f32,
    /// And the acceleration, separately — at `1` a higher speed takes as long to reach and reads
    /// sluggish.
    pub eagerness: f32,
    /// Last step's `wanted`, so a toggle hears presses rather than holds.
    #[reflect(skip)]
    pub was_wanted: bool,
    #[reflect(skip)]
    pub running: bool,
}

impl Default for Sprint {
    fn default() -> Self {
        Self {
            wanted: false,
            toggle: false,
            speed: 1.8,
            eagerness: 1.4,
            was_wanted: false,
            running: false,
        }
    }
}

impl Component for Sprint {}

impl Sprint {
    /// Advances one step. Only moving can keep a toggled run going: stopping is how it ends.
    pub fn step(&mut self, moving: bool) {
        let pressed = self.wanted && !self.was_wanted;
        self.was_wanted = self.wanted;
        self.running = match self.toggle {
            true => moving && (self.running != pressed),
            false => self.wanted,
        };
    }

    /// This sprint's scaling, or none at all when it is not running.
    pub fn scale(&self) -> (f32, f32) {
        match self.running {
            true => (self.speed.max(0.0), self.eagerness.max(0.0)),
            false => (1.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests;
