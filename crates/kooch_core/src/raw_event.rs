//! Raw event forwarding for overlay systems.
//!
//! Provides the [`RawEventHandler`] trait that allows external systems
//! (e.g., egui overlay) to receive raw windowing events for input
//! processing without coupling `kooch_core` to any windowing library.

use std::any::Any;

/// Handler for raw windowing events.
///
/// Stored as `Box<dyn RawEventHandler>` in [`Resources`](crate::resource::Resources).
/// The window system calls [`on_event`](Self::on_event) for every window event
/// before frame processing.
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
    /// Returns `true` if the event was consumed and should not propagate.
    fn on_event(&mut self, window: &dyn Any, event: &dyn Any) -> bool;
}
