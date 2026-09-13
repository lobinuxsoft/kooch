//! Raw event forwarding for overlay systems.

use std::any::Any;

/// Handler for raw windowing events.
pub trait RawEventHandler: Send + Sync + 'static {
    /// Process a raw window event.
    fn on_event(&mut self, window: &dyn Any, event: &dyn Any) -> bool;
}

/// Every handler interested in raw window events, in dispatch order.
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
