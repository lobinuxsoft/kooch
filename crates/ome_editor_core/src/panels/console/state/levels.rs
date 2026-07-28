//! Which severities the Console is showing.
//!
//! # Why a set and not a threshold
//!
//! "At least this severe" cannot express the thing anyone actually wants
//! when a panel is drowning: *hide the warnings, keep everything else*.
//! A threshold at `info` drags every warning along with it, and a
//! threshold at `error` throws away the line being looked for.
//!
//! That came up with #641, where three hundred repeats of one egui warning
//! buried every other line in the log. With a threshold there was no way
//! to read past them.

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
///
/// A `u8` rather than five `bool`s so it compares and copies as one value
/// — the filtered view is rebuilt by comparing the settings it was built
/// from, and a set that is one number makes that comparison exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LevelSet(u8);

impl LevelSet {
    /// Errors, warnings and info: what the old `INFO` threshold showed.
    ///
    /// Debug and trace stay off because the engine is genuinely noisy at
    /// those levels — a hundred asset lines before the one that matters.
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
mod tests {
    use super::*;

    #[test]
    fn the_default_shows_errors_warnings_and_info() {
        let set = LevelSet::default();
        assert!(set.shows(Level::ERROR));
        assert!(set.shows(Level::WARN));
        assert!(set.shows(Level::INFO));
        assert!(!set.shows(Level::DEBUG));
        assert!(!set.shows(Level::TRACE));
    }

    /// The whole point: hiding one severity leaves the others alone. A
    /// threshold could not do this, which is why #641's three hundred
    /// warnings buried everything else.
    #[test]
    fn hiding_warnings_keeps_info_and_errors() {
        let mut set = LevelSet::default();
        set.set(Level::WARN, false);

        assert!(!set.shows(Level::WARN));
        assert!(set.shows(Level::INFO), "info went with it");
        assert!(set.shows(Level::ERROR), "errors went with it");
    }

    #[test]
    fn a_level_can_be_turned_back_on() {
        let mut set = LevelSet::default();
        set.set(Level::TRACE, true);
        assert!(set.shows(Level::TRACE));
        set.set(Level::TRACE, false);
        assert!(!set.shows(Level::TRACE));
    }

    #[test]
    fn every_level_has_its_own_bit() {
        for level in ALL {
            let mut set = LevelSet(0);
            set.set(level, true);
            for other in ALL {
                assert_eq!(
                    set.shows(other),
                    other == level,
                    "{level} and {other} share a bit",
                );
            }
        }
    }

    #[test]
    fn turning_everything_off_is_visible_as_such() {
        let mut set = LevelSet::default();
        assert!(!set.is_empty());
        for level in ALL {
            set.set(level, false);
        }
        assert!(set.is_empty());
    }
}
