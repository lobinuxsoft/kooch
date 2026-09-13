//! Window events, delivered through the engine's double-buffered event system: readable the frame
//! after they are sent.

/// Emitted when the window is resized.
///
/// Contains the new inner size in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowResized {
    /// New width in physical pixels.
    pub width: u32,
    /// New height in physical pixels.
    pub height: u32,
}

/// Emitted when the user asks to close the window (X button, Alt+F4).
/// [`WindowPlugin`](crate::WindowPlugin) also sends [`AppExit`](kooch_core::event::AppExit) when it
/// fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowCloseRequested;
