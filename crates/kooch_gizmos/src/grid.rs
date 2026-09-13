//! Which grid level to draw, and how far through the crossfade it is.

/// How many fine cells make one counting cell. Ten, because a person counts in decimals.
pub const STEPS: f32 = 10.0;

/// The grid level a camera at this distance should see.
/// `log(distance) / log(steps)` keeps cells about one size on screen; its fraction is how far the
/// next level has crossfaded in (after Godot, MIT).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLevel {
    /// The finer of the two levels being blended, in world units.
    pub small_step: f32,
    /// 0 at the start of a level, 1 as the next one takes over.
    pub blend: f32,
}

impl GridLevel {
    /// The level for a camera `distance` from the plane, never finer than `step` — the value
    /// handles snap to.
    pub fn at(distance: f32, step: f32) -> Self {
        if step <= 0.0 || !distance.is_finite() {
            return Self {
                small_step: step.max(f32::EPSILON),
                blend: 0.0,
            };
        }

        // In units of the snap step, so level 0 IS the step rather than
        // one metre. A project working in centimetres gets the same
        // behaviour as one working in metres.
        let level = (distance.abs().max(step) / step).log(STEPS);
        let floored = level.floor();
        Self {
            small_step: step * STEPS.powf(floored),
            blend: level - floored,
        }
    }

    /// A level pinned to `step` whatever the camera does: a guide shows the scale a drag moves in.
    pub fn fixed(step: f32) -> Self {
        Self {
            small_step: step.max(f32::EPSILON),
            blend: 0.0,
        }
    }

    /// The coarser level, which the fine one fades into.
    pub fn large_step(self) -> f32 {
        self.small_step * STEPS
    }
}

#[cfg(test)]
mod tests;
