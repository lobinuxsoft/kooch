//! Input identifiers owned by the engine, not winit's or gilrs': `gilrs::GamepadId` cannot be
//! constructed by a remote host (#710), and saved bindings must not depend on winit.
//! Names mirror winit and gilrs; one macro list derives each enum and both conversions.

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
            /// Every variant in declaration order, for the binding picker — generated so no second
            /// list can drift.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];

            /// The upstream value this mirrors, or `None` for one with no name here — not a
            /// catch-all `Unknown` that would make them all equal.
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
    /// A gamepad button by position on an SDL-layout pad: `South` is A, Cross or B, so one binding
    /// means confirm everywhere.
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
    /// A mouse button; not generated, because upstream's `Other(u16)` carries a payload.
    MouseButton <=> winit::event::MouseButton {
    Left, Right, Middle, Back, Forward,
    }
}

/// Which gamepad, as the engine numbers them — a plain number a remote host can name.
/// Session-scoped: never written to a file (#55).
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
mod tests;

#[cfg(test)]
mod all_tests;
