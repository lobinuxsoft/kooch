//! Editor viewport — offscreen render target + ray-march pass feeding the
//! View panel through `egui_wgpu::Renderer::register_native_texture`.

mod render;
mod target;

pub(crate) use render::render_viewport;
pub(crate) use target::ViewportTarget;
