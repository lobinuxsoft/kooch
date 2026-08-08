//! Raw event forwarding for overlay systems.
//!
//! Provides the [`RawEventHandler`] trait that allows external systems
//! (e.g., egui overlay) to receive raw windowing events for input
//! processing without coupling `kooch_core` to any windowing library.

use std::any::Any;

/// Handler for raw windowing events.
///
/// Registered by pushing into [`RawEventHandlers`], which the window
/// system stores in [`Resources`](crate::resource::Resources) and calls
/// for every window event before frame processing.
///
/// Both parameters are type-erased to avoid coupling `kooch_core` to any
/// specific windowing library. Implementations should downcast to the
/// expected concrete types (e.g., `winit::window::Window` and
/// `winit::event::WindowEvent`).
pub trait RawEventHandler: Send + Sync + 'static {
    /// Process a raw window event.
    ///
    /// # Parameters
    /// - `window`: the window reference (e.g., `&winit::window::Window`)
    /// - `event`: the event (e.g., `&winit::event::WindowEvent`)
    ///
    /// Returns `true` if the event was consumed and should not propagate
    /// to handlers registered after this one.
    fn on_event(&mut self, window: &dyn Any, event: &dyn Any) -> bool;
}

/// Every handler interested in raw window events, in dispatch order.
///
/// # Why a list, and why it is ordered
///
/// There is more than one thing that wants the keyboard. The editor's
/// egui overlay wants it, and so does gameplay input — and when a text
/// field has focus, typing `w` must *not* also drive the player forward.
/// A single handler slot answered that by leaving the second interested
/// party with nothing at all: whoever inserted last silently replaced
/// whoever inserted first.
///
/// So handlers dispatch in registration order and the first one to
/// return `true` ends the event. That return value used to be discarded
/// by the caller, which made "consumed" a word in a doc comment rather
/// than a behaviour.
#[derive(Default)]
pub struct RawEventHandlers {
    handlers: Vec<Box<dyn RawEventHandler>>,
}

impl RawEventHandlers {
    /// Creates an empty list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a handler. It sees events after every handler already
    /// registered, and only those none of them consumed.
    pub fn push(&mut self, handler: Box<dyn RawEventHandler>) {
        self.handlers.push(handler);
    }

    /// Offers the event to each handler in turn, stopping at the first
    /// that consumes it. Returns whether any did.
    pub fn dispatch(&mut self, window: &dyn Any, event: &dyn Any) -> bool {
        for handler in &mut self.handlers {
            if handler.on_event(window, event) {
                return true;
            }
        }
        false
    }

    /// Number of registered handlers.
    pub fn len(&self) -> usize {
        self.handlers.len()
    }

    /// Returns `true` when nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }
}

#[cfg(test)]
mod tests;
