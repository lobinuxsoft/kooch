//! Window events emitted by the windowing system.
//!
//! These events are sent through the engine's double-buffered event system
//! and become readable on the next frame.

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

/// Emitted when the user requests the window to close (X button, Alt+F4, etc.).
///
/// Systems can listen for this event to perform cleanup before shutdown.
/// By default, [`WindowPlugin`](crate::WindowPlugin) also sends [`AppExit`](ome_core::event::AppExit)
/// when this event fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowCloseRequested;
