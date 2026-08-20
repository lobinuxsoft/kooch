use super::{WindowMode, mode_from};

#[test]
fn unset_has_no_opinion() {
    assert_eq!(mode_from(None), None);
}

#[test]
fn the_three_modes_are_read() {
    assert_eq!(mode_from(Some("windowed")), Some(WindowMode::Windowed));
    assert_eq!(mode_from(Some("borderless")), Some(WindowMode::Borderless));
    assert_eq!(
        mode_from(Some(" fullscreen ")),
        Some(WindowMode::Fullscreen)
    );
}

/// 🔴 A typo must not take the display, and must not overwrite the
/// author's choice with a guess either.
#[test]
fn a_typo_has_no_opinion() {
    assert_eq!(mode_from(Some("full-screen")), None);
    assert_eq!(mode_from(Some("2")), None);
    assert_eq!(mode_from(Some("")), None);
}

/// The numbers are serialised into user projects, so they are
/// append-only and an unknown one falls back to the mode that is always
/// available rather than to the one that takes the screen.
#[test]
fn unknown_numbers_stay_windowed() {
    assert_eq!(
        WindowMode::resolve(WindowMode::Borderless, None),
        WindowMode::Borderless
    );
    assert_eq!(WindowMode::from_asset(0), WindowMode::Windowed);
    assert_eq!(WindowMode::from_asset(99), WindowMode::Windowed);
}

#[test]
fn the_variable_outranks_the_asset() {
    let over = Some(WindowMode::Windowed);
    assert_eq!(
        WindowMode::resolve(WindowMode::Fullscreen, over),
        WindowMode::Windowed
    );
    assert_eq!(
        WindowMode::resolve(WindowMode::Windowed, Some(WindowMode::Fullscreen)),
        WindowMode::Fullscreen,
    );
}

/// 🔴 Borderless is a WINDOW without a border, not a fullscreen one —
/// the distinction the name invites people to lose.
#[test]
fn borderless_is_not_fullscreen() {
    assert!(!WindowMode::Borderless.fullscreen());
    assert!(!WindowMode::Borderless.decorated());
    assert!(WindowMode::Fullscreen.fullscreen());
    assert!(WindowMode::Windowed.decorated());
    assert!(!WindowMode::Windowed.fullscreen());
}
