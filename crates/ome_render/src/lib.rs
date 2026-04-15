//! ome_render — renderers for oh_my_engine.
//!
//! - [`RenderPlugin`] clears the screen with a solid [`ClearColor`] each
//!   frame (used as the minimal GPU smoke test).
//! - [`RayMarchPlugin`] sphere-traces SDF components from the ECS into
//!   a fullscreen fragment shader.

pub mod clear_color;
pub mod fps;
pub mod plugin;
pub mod raymarch;
pub mod raymarch_plugin;
mod systems;

pub use clear_color::ClearColor;
pub use fps::FpsTracker;
pub use plugin::RenderPlugin;
pub use raymarch::{RayMarchParams, RayMarchRenderer};
pub use raymarch_plugin::{RayMarchPlugin, SkyGradient};
