//! [`ControlPath`] — what a binding points at.
//!
//! # Why this is not a device
//!
//! The binding this replaces was `InputBinding::GamepadButton(GamepadId,
//! GamepadButton)`, and that first field is the pad's index **for this
//! session**. Bind jump to it with one controller plugged in, unplug it,
//! and the binding now names a pad that is not there — or worse, a
//! different one. Written to disk it is a number that means nothing next
//! time.
//!
//! A binding names a **class of device**, and which physical device
//! satisfies it is resolved every frame. That is why plugging a
//! controller in mid-game just works, and it is the same conclusion
//! Unity reached: their paths read `<Gamepad>/buttonSouth`, where
//! `<Gamepad>` is a layout and not a device.
//!
//! # Why an enum and not their string
//!
//! Unity stores the path as text and parses it. That buys wildcards
//! (`<Gamepad>/button*`) and usage tags (`*/{Submit}`) — real features we
//! do not need yet — at the price of every typo becoming a binding that
//! silently matches nothing.
//!
//! Ours is a closed set, checked by the compiler, and the editor's picker
//! offers exactly the variants that exist. If wildcards ever earn their
//! place, a `Pattern` variant can carry them without turning the other
//! ninety-nine percent into text.

use serde::{Deserialize, Serialize};

use crate::ids::{GamepadAxis, GamepadButton, KeyCode, MouseButton};

/// A control a binding reads, named by kind rather than by device.
///
/// Deliberately carries no device id. "The south button on any gamepad"
/// is the useful thing to author; "the south button on pad 2" is a
/// filter, and belongs to control schemes (#60) rather than here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ControlPath {
    /// A keyboard key, by physical position — `KeyA` is the key left of
    /// `KeyS` whatever the layout prints on it.
    Key(KeyCode),
    /// A mouse button.
    Mouse(MouseButton),
    /// A gamepad button, on whichever pad is answering.
    Button(GamepadButton),
    /// A gamepad axis, on whichever pad is answering.
    Axis(GamepadAxis),
}

impl ControlPath {
    /// Whether this control is on/off rather than continuous.
    ///
    /// An axis read as a button is "past halfway", and a button read as
    /// an axis is 0 or 1 — the two are always convertible, so this only
    /// decides which reading is the natural one.
    pub fn is_digital(self) -> bool {
        matches!(
            self,
            ControlPath::Key(_) | ControlPath::Mouse(_) | ControlPath::Button(_)
        )
    }

    /// The device class this control belongs to.
    pub fn device(self) -> DeviceClass {
        match self {
            ControlPath::Key(_) => DeviceClass::Keyboard,
            ControlPath::Mouse(_) => DeviceClass::Mouse,
            ControlPath::Button(_) | ControlPath::Axis(_) => DeviceClass::Gamepad,
        }
    }
}

/// The kind of device a control lives on.
///
/// Enough to answer "is this player on a controller right now", which is
/// what drives prompt glyphs and is the only thing gameplay legitimately
/// asks about the hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceClass {
    Keyboard,
    Mouse,
    Gamepad,
}

#[cfg(test)]
mod tests;
