//! Window handle resource for sharing the winit window.
//!
//! [`WindowHandle`] wraps an `Arc<Window>` so downstream systems (wgpu, input)
//! can access the window without ownership concerns.

use std::sync::Arc;

use winit::window::Window;

/// A clonable handle to the winit window.
///
/// Inserted as a resource after the window is created in `resumed()`.
/// Systems that need the window (e.g., wgpu surface creation) can read this
/// from resources.
///
/// # Example
/// ```ignore
/// fn setup_renderer(resources: &mut Resources) {
///     let handle = resources.get::<WindowHandle>().unwrap();
///     let surface = instance.create_surface(handle.window().clone()).unwrap();
/// }
/// ```
#[derive(Clone)]
pub struct WindowHandle {
    window: Arc<Window>,
}

impl WindowHandle {
    /// Creates a new window handle.
    pub(crate) fn new(window: Arc<Window>) -> Self {
        Self { window }
    }

    /// Returns a reference to the underlying `Arc<Window>`.
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// Returns the inner size of the window in physical pixels.
    pub fn inner_size(&self) -> (u32, u32) {
        let size = self.window.inner_size();
        (size.width, size.height)
    }
}
