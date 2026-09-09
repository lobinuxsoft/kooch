//! Which grid level to draw, and how far through the crossfade it is.

/// How many fine cells make one counting cell.
///
/// Ten, because a decimal lattice is what a person counts in. Godot
/// exposes it; nothing here has ever wanted another value.
pub const STEPS: f32 = 10.0;

/// The level of grid a camera at this distance should see.
///
/// # The whole point is the fractional part
///
/// A grid with one fixed step is either invisible from far away or a
/// sheet of moiré from close up. Choosing a level by
/// `log(distance) / log(steps)` keeps the cells roughly one size on
/// screen at every zoom — and the **fraction** left over is how far the
/// next level has taken over, so the change is a crossfade rather than
/// a pop.
///
/// Adapted from Godot's `_init_grid` (MIT), which does the same and
/// then applies the fraction per line on the CPU because its grid is
/// geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLevel {
    /// The finer of the two levels being blended, in world units.
    pub small_step: f32,
    /// 0 at the start of a level, 1 as the next one takes over.
    pub blend: f32,
}

impl GridLevel {
    /// The level for a camera `distance` from the plane, snapped so the
    /// finest it ever shows is `step`.
    ///
    /// `step` is the value the handles snap to, so the grid can never
    /// draw a cell finer than one an author can actually land on.
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

    /// The coarser level, which the fine one fades into.
    pub fn large_step(self) -> f32 {
        self.small_step * STEPS
    }
}

#[cfg(test)]
mod tests;
