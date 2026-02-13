//! ome_render — clear-color renderer for oh_my_engine.
//!
//! Provides [`RenderPlugin`] which clears the screen with a solid
//! [`ClearColor`] each frame, verifying the full GPU pipeline works.

pub mod clear_color;
pub mod fps;
pub mod plugin;
mod systems;

pub use clear_color::ClearColor;
pub use fps::FpsTracker;
pub use plugin::RenderPlugin;
