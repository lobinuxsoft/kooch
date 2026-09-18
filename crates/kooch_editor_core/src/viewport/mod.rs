//! Editor viewport — offscreen render target + ray-march pass feeding the
//! View panel through `egui_wgpu::Renderer::register_native_texture`.

pub(crate) mod game;
pub(crate) mod post;
pub(crate) mod render;
pub(crate) mod shader_preview;
mod target;

pub(crate) use game::{GameView, render_game_view};
pub(crate) use render::render_viewport;
pub(crate) use shader_preview::{PreviewRequest, ShaderPreview};
pub(crate) use target::ViewportTarget;
