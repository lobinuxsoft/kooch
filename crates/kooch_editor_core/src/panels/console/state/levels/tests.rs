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
