//! [`ControlPath`] — what a binding points at: a class of device, resolved every frame, never a
//! session's pad index, so plugging a controller in just works.
//! A closed enum, not Unity's parsed strings, so a typo cannot bind to nothing.

use serde::{Deserialize, Serialize};

use crate::ids::{GamepadAxis, GamepadButton, KeyCode, MouseAxis, MouseButton};

/// A control a binding reads, by kind and not by device; which pad is a control-scheme concern
/// (#60).
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
    /// The mouse's motion along one axis, as a **velocity** in pixels per second (#1266). A rate, as
    /// a stick is: one action binds both and is read with one formula, and it survives the wire,
    /// where the editor and the project do not share a frame rate.
    MouseMotion(MouseAxis),
}

impl ControlPath {
    #[cfg(test)]
    /// Whether this control is on/off rather than continuous — the natural reading, since axis and
    /// button always convert.
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
            ControlPath::Mouse(_) | ControlPath::MouseMotion(_) => DeviceClass::Mouse,
            ControlPath::Button(_) | ControlPath::Axis(_) => DeviceClass::Gamepad,
        }
    }
}

/// The kind of device a control lives on — enough for prompt glyphs, the only hardware question
/// gameplay should ask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceClass {
    Keyboard,
    Mouse,
    Gamepad,
}

#[cfg(test)]
mod tests;
