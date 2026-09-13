//! Shared handle to the winit window, so wgpu and input can reach it without owning it.

use std::sync::Arc;

use winit::window::Window;

/// A clonable handle to the winit window, inserted as a resource once `resumed()` creates it.
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
