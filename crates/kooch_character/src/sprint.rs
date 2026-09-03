//! [`Sprint`] — running, as a multiplier on walking.

use kooch_ecs::Reflect;
use kooch_ecs::component::Component;

/// Running: [`Walk`](crate::Walk) with its numbers scaled while
/// `wanted` is set.
///
/// # Why this has no system of its own
///
/// It applies no force. Everything a sprint does, walking already does
/// — it only does it faster — so a separate system would have to
/// duplicate the goal, the chase and the cap to change two numbers.
/// A mechanic that adds a *term* gets a system; one that scales an
/// existing term is a modifier, and this is one.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct Sprint {
    /// Written by gameplay every frame, like
    /// [`Facing`](crate::Facing): held is running, released is not.
    pub wanted: bool,
    /// What the top speed is multiplied by.
    pub speed: f32,
    /// And the acceleration, separately.
    ///
    /// Left at `1` a sprint takes as long to reach a higher speed as
    /// walking took to reach a lower one, which reads as sluggish. Above
    /// it the character snaps to running.
    pub eagerness: f32,
}

impl Default for Sprint {
    fn default() -> Self {
        Self {
            wanted: false,
            speed: 1.8,
            eagerness: 1.4,
        }
    }
}

impl Component for Sprint {}

impl Sprint {
    /// This sprint's scaling, or none at all when it is not asked for.
    pub fn scale(&self) -> (f32, f32) {
        match self.wanted {
            true => (self.speed.max(0.0), self.eagerness.max(0.0)),
            false => (1.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests;
