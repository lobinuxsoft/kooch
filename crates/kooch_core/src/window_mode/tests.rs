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

mod resolutions {
    use crate::window_mode::{Resolution, WindowMode, best_mode, effective};

    fn res(width: u32, height: u32, refresh_mhz: u32) -> Resolution {
        Resolution {
            width,
            height,
            refresh_mhz,
        }
    }

    fn monitor() -> Vec<Resolution> {
        vec![
            res(1920, 1080, 144_000),
            res(1920, 1080, 60_000),
            res(1280, 720, 60_000),
        ]
    }

    /// 🔴 Winit warns and changes nothing when Wayland is handed an
    /// exclusive request, which leaves the window as it was and reads as
    /// the setting being broken. Degrading here means the player gets
    /// the closest thing that works.
    #[test]
    fn wayland_gets_borderless_instead() {
        assert_eq!(
            effective(WindowMode::Exclusive, false),
            WindowMode::Fullscreen
        );
        assert_eq!(
            effective(WindowMode::Exclusive, true),
            WindowMode::Exclusive
        );
    }

    /// Only the exclusive request is affected — a downgrade that touched
    /// the others would take a windowed game full screen.
    #[test]
    fn the_other_modes_pass_through() {
        for mode in [
            WindowMode::Windowed,
            WindowMode::Borderless,
            WindowMode::Fullscreen,
        ] {
            assert_eq!(effective(mode, false), mode);
            assert_eq!(effective(mode, true), mode);
        }
    }

    /// No refresh asked for means "the best this size can do".
    #[test]
    fn no_refresh_takes_the_highest() {
        assert_eq!(
            best_mode(&monitor(), res(1920, 1080, 0)),
            Some(res(1920, 1080, 144_000)),
        );
    }

    /// A refresh that was asked for wins over the highest one.
    #[test]
    fn an_asked_refresh_is_honoured() {
        assert_eq!(
            best_mode(&monitor(), res(1920, 1080, 60_000)),
            Some(res(1920, 1080, 60_000)),
        );
        // Nearest, not exact: a monitor reporting 59.94 Hz must not be
        // refused because the list said 60.
        assert_eq!(
            best_mode(&monitor(), res(1920, 1080, 59_940)),
            Some(res(1920, 1080, 60_000)),
        );
    }

    /// 🔴 The SIZE has to match exactly. Picking a nearby resolution
    /// would change what the player sees without saying so; the honest
    /// answer is none, and the caller stays at borderless.
    #[test]
    fn a_missing_size_is_none() {
        assert!(best_mode(&monitor(), res(1600, 900, 0)).is_none());
        assert!(best_mode(&[], res(1920, 1080, 0)).is_none());
    }
}
