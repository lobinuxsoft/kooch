//! [`ActiveOrigin`] — the universe coordinate that defines `(0, 0, 0)` of the simulation frame the
//! rest of the engine operates in.

use crate::coord::UniverseCoord;

/// Universe coordinate the simulation frame is anchored at.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActiveOrigin {
    coord: UniverseCoord,
}

impl ActiveOrigin {
    pub const ZERO: Self = Self {
        coord: UniverseCoord::ZERO,
    };

    pub const fn new(coord: UniverseCoord) -> Self {
        Self { coord }
    }

    /// The universe coordinate currently treated as `(0, 0, 0)` of the
    /// simulation frame.
    pub fn coord(&self) -> UniverseCoord {
        self.coord
    }

    /// Replace the active origin. Does **not** shift any entities — the caller is responsible for
    /// issuing the matching delta application to `Query<&mut GlobalTransform>` (or equivalent) to
    /// keep world positions invariant. Prefer the rebase system over manual sets.
    pub fn set(&mut self, coord: UniverseCoord) {
        self.coord = coord;
    }
}

#[cfg(test)]
mod tests;
