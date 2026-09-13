//! Which severities the Console is showing.

use tracing::Level;

/// The five severities, in the order they are shown.
pub(crate) const ALL: [Level; 5] = [
    Level::ERROR,
    Level::WARN,
    Level::INFO,
    Level::DEBUG,
    Level::TRACE,
];

/// The set of severities the Console shows, one bit each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LevelSet(u8);

impl LevelSet {
    /// Errors, warnings and info: what the old `INFO` threshold showed.
    pub(crate) const DEFAULT: Self = Self(0b0000_0111);

    /// The bit for one level.
    const fn bit(level: Level) -> u8 {
        // `tracing::Level` has no index, and matching keeps this readable
        // where an arithmetic trick would not.
        match level {
            Level::ERROR => 1 << 0,
            Level::WARN => 1 << 1,
            Level::INFO => 1 << 2,
            Level::DEBUG => 1 << 3,
            _ => 1 << 4,
        }
    }

    /// Whether `level` is shown.
    pub(crate) const fn shows(self, level: Level) -> bool {
        self.0 & Self::bit(level) != 0
    }

    /// Turns `level` on or off.
    pub(crate) const fn set(&mut self, level: Level, on: bool) {
        match on {
            true => self.0 |= Self::bit(level),
            false => self.0 &= !Self::bit(level),
        }
    }

    /// Whether nothing at all is shown — worth saying out loud in the UI,
    /// because an empty Console otherwise reads as "nothing happened".
    pub(crate) const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl Default for LevelSet {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests;
