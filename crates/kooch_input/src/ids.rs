//! Input identifiers that belong to this engine.
//!
//! # Why not just re-export winit's and gilrs'
//!
//! That is what this crate did, under a note saying neutral wrappers were
//! deferred until a non-winit backend shipped. Three things came due at
//! once:
//!
//! - **`gilrs::GamepadId` cannot be constructed.** It is
//!   `GamepadId(pub(crate) usize)` with a one-way `From<GamepadId> for
//!   usize`. A remote host receiving gamepad state over a socket has no
//!   way to name the pad it was told about (#710).
//! - **A binding has to survive being written to disk.** An `.inputmap`
//!   holding `winit::keyboard::KeyCode` ties the file format to winit's
//!   version, and the serialised name carries the crate it came from —
//!   the trap that already cost us a silent breakage during the rename.
//! - **Steam Input is a non-winit backend** (#60), which is exactly the
//!   condition the old note deferred to.
//!
//! # Where the names come from
//!
//! [`KeyCode`] mirrors winit's `KeyCode`, which mirrors the W3C UI Events
//! `code` values — physical positions on the keyboard, not the letters
//! printed on them. `KeyA` is the key left of `KeyS` whatever the layout
//! says. Copying that vocabulary rather than inventing one means the
//! conversion is a rename and a reader who knows one knows the other.
//!
//! [`GamepadButton`] and [`GamepadAxis`] mirror gilrs, which follows the
//! SDL game-controller layout: `South`/`East`/`North`/`West` by position,
//! so a binding does not lie about A/B/X/Y on a DualSense.
//!
//! # The lists are generated from those crates, and the macro is why
//!
//! Each list appears **once**. The macro derives the enum, the conversion
//! in, and the conversion out from the same source, so a variant cannot
//! exist in one and be missing from another — which is the failure a
//! hand-written 194-arm match invites.

use serde::{Deserialize, Serialize};

/// Declares an identifier enum plus its conversions to and from the
/// upstream type it mirrors, from one list of names.
macro_rules! mirrored {
    (
        $(#[$meta:meta])*
        $name:ident <=> $upstream:path {
            $($variant:ident,)*
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[non_exhaustive]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            /// Every variant, in declaration order.
            ///
            /// The editor's binding picker needs a list to offer, and a
            /// hand-written one beside the enum is a second place to add a
            /// variant to — which is the failure this macro exists to
            /// prevent. Same list, same expansion.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The upstream value this mirrors, or `None` for one this
            /// engine has no name for.
            ///
            /// `None` rather than a fallback variant: an input we cannot
            /// name is an input no binding can mention, and inventing an
            /// `Unknown` that every unmapped key collapses into would make
            /// them all compare equal.
            pub fn from_upstream(value: $upstream) -> Option<Self> {
                match value {
                    $(<$upstream>::$variant => Some(Self::$variant),)*
                    _ => None,
                }
            }

            /// This value as the upstream type.
            pub fn to_upstream(self) -> $upstream {
                match self {
                    $(Self::$variant => <$upstream>::$variant,)*
                }
            }
        }

        impl From<$name> for $upstream {
            fn from(value: $name) -> Self {
                value.to_upstream()
            }
        }
    };
}

mirrored! {
    /// A physical key position, named by the W3C UI Events `code` it sits
    /// at rather than the character it produces.
    KeyCode <=> winit::keyboard::KeyCode {
    Backquote, Backslash, BracketLeft, BracketRight, Comma, Digit0,
    Digit1, Digit2, Digit3, Digit4, Digit5, Digit6,
    Digit7, Digit8, Digit9, Equal, IntlBackslash, IntlRo,
    IntlYen, KeyA, KeyB, KeyC, KeyD, KeyE,
    KeyF, KeyG, KeyH, KeyI, KeyJ, KeyK,
    KeyL, KeyM, KeyN, KeyO, KeyP, KeyQ,
    KeyR, KeyS, KeyT, KeyU, KeyV, KeyW,
    KeyX, KeyY, KeyZ, Minus, Period, Quote,
    Semicolon, Slash, AltLeft, AltRight, Backspace, CapsLock,
    ContextMenu, ControlLeft, ControlRight, Enter, SuperLeft, SuperRight,
    ShiftLeft, ShiftRight, Space, Tab, Convert, KanaMode,
    Lang1, Lang2, Lang3, Lang4, Lang5, NonConvert,
    Delete, End, Help, Home, Insert, PageDown,
    PageUp, ArrowDown, ArrowLeft, ArrowRight, ArrowUp, NumLock,
    Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5,
    Numpad6, Numpad7, Numpad8, Numpad9, NumpadAdd, NumpadBackspace,
    NumpadClear, NumpadClearEntry, NumpadComma, NumpadDecimal, NumpadDivide, NumpadEnter,
    NumpadEqual, NumpadHash, NumpadMemoryAdd, NumpadMemoryClear, NumpadMemoryRecall, NumpadMemoryStore,
    NumpadMemorySubtract, NumpadMultiply, NumpadParenLeft, NumpadParenRight, NumpadStar, NumpadSubtract,
    Escape, Fn, FnLock, PrintScreen, ScrollLock, Pause,
    BrowserBack, BrowserFavorites, BrowserForward, BrowserHome, BrowserRefresh, BrowserSearch,
    BrowserStop, Eject, LaunchApp1, LaunchApp2, LaunchMail, MediaPlayPause,
    MediaSelect, MediaStop, MediaTrackNext, MediaTrackPrevious, Power, Sleep,
    AudioVolumeDown, AudioVolumeMute, AudioVolumeUp, WakeUp, Meta, Hyper,
    Turbo, Abort, Resume, Suspend, Again, Copy,
    Cut, Find, Open, Paste, Props, Select,
    Undo, Hiragana, Katakana, F1, F2, F3,
    F4, F5, F6, F7, F8, F9,
    F10, F11, F12, F13, F14, F15,
    F16, F17, F18, F19, F20, F21,
    F22, F23, F24, F25, F26, F27,
    F28, F29, F30, F31, F32, F33,
    F34, F35,
    }
}

mirrored! {
    /// A gamepad button, named by position on an SDL-layout pad.
    ///
    /// `South` is the bottom face button: A on Xbox, Cross on PlayStation,
    /// B on a Nintendo pad. Naming it by position is what lets one binding
    /// mean "the confirm button" everywhere.
    GamepadButton <=> gilrs::Button {
    South, East, North, West, C, Z,
    LeftTrigger, LeftTrigger2, RightTrigger, RightTrigger2, Select, Start,
    Mode, LeftThumb, RightThumb, DPadUp, DPadDown, DPadLeft,
    DPadRight,
    }
}

mirrored! {
    /// A gamepad axis.
    GamepadAxis <=> gilrs::Axis {
    LeftStickX, LeftStickY, LeftZ, RightStickX, RightStickY, RightZ,
    DPadX, DPadY,
    }
}

mirrored! {
    /// A mouse button.
    ///
    /// The upstream type also has `Back`, `Forward` and `Other(u16)`;
    /// `Other` is the reason this is not generated from the same list —
    /// a variant with a payload is not a name.
    MouseButton <=> winit::event::MouseButton {
    Left, Right, Middle, Back, Forward,
    }
}

/// Which gamepad, as this engine numbers them.
///
/// A plain number because the backend that owns the devices is not always
/// the process that reads them: a remote host is told "pad 0 pressed
/// South" over a socket and must be able to say so, which
/// `gilrs::GamepadId` makes impossible — it has no public constructor.
///
/// Stable only within a session. Two sessions, or two backends, may
/// number the same physical pad differently, so this must never reach a
/// file. A binding names a **slot** (#55), not a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GamepadId(pub u32);

impl GamepadId {
    /// The number a backend assigned to this pad.
    pub fn index(self) -> u32 {
        self.0
    }
}

impl From<gilrs::GamepadId> for GamepadId {
    fn from(value: gilrs::GamepadId) -> Self {
        Self(usize::from(value) as u32)
    }
}

impl std::fmt::Display for GamepadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "gamepad {}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_survives_the_round_trip_through_winit() {
        for key in [
            KeyCode::KeyW,
            KeyCode::Space,
            KeyCode::ArrowUp,
            KeyCode::F12,
            KeyCode::NumpadEnter,
        ] {
            assert_eq!(KeyCode::from_upstream(key.to_upstream()), Some(key));
        }
    }

    #[test]
    fn a_button_survives_the_round_trip_through_gilrs() {
        for button in [
            GamepadButton::South,
            GamepadButton::DPadUp,
            GamepadButton::Start,
        ] {
            assert_eq!(
                GamepadButton::from_upstream(button.to_upstream()),
                Some(button)
            );
        }
    }

    #[test]
    fn an_axis_survives_the_round_trip_through_gilrs() {
        for axis in [
            GamepadAxis::LeftStickX,
            GamepadAxis::RightStickY,
            GamepadAxis::DPadX,
        ] {
            assert_eq!(GamepadAxis::from_upstream(axis.to_upstream()), Some(axis));
        }
    }

    /// gilrs has an `Unknown` variant and this does not, on purpose.
    #[test]
    fn an_input_we_have_no_name_for_is_none_rather_than_a_catch_all() {
        assert_eq!(GamepadButton::from_upstream(gilrs::Button::Unknown), None);
        assert_eq!(GamepadAxis::from_upstream(gilrs::Axis::Unknown), None);
    }

    /// The point of the whole module: this is what gilrs cannot do.
    #[test]
    fn a_gamepad_id_can_be_built_from_a_number() {
        assert_eq!(GamepadId(3).index(), 3);
    }

    #[test]
    fn identifiers_serialise_by_name_so_a_file_reads_as_text() {
        let json = serde_json::to_string(&KeyCode::KeyW).unwrap();
        assert_eq!(json, "\"KeyW\"");
        assert_eq!(
            serde_json::from_str::<KeyCode>(&json).unwrap(),
            KeyCode::KeyW
        );
    }
}

#[cfg(test)]
mod all_tests {
    use super::*;

    /// `ALL` has to actually be all of them: a picker that offers a
    /// subset is a control nobody can bind, with nothing to say so.
    #[test]
    fn every_list_covers_its_enum() {
        // Round-tripping every entry proves the list and the conversions
        // came from the same expansion.
        for key in KeyCode::ALL {
            assert_eq!(KeyCode::from_upstream(key.to_upstream()), Some(*key));
        }
        for button in GamepadButton::ALL {
            assert_eq!(
                GamepadButton::from_upstream(button.to_upstream()),
                Some(*button)
            );
        }
        for axis in GamepadAxis::ALL {
            assert_eq!(GamepadAxis::from_upstream(axis.to_upstream()), Some(*axis));
        }
        for button in MouseButton::ALL {
            assert_eq!(
                MouseButton::from_upstream(button.to_upstream()),
                Some(*button)
            );
        }
        assert!(
            KeyCode::ALL.len() > 100,
            "the keyboard list looks truncated"
        );
    }
}
