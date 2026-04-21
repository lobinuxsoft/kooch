//! Editor startup system — initializes egui, winit integration and wgpu renderer.

use std::sync::Arc;

use ome_core::gpu::GpuContext;
use ome_core::raw_event::RawEventHandler;
use ome_core::resource::Resources;

use crate::state::{EditorOverlay, EguiEventHandler};
use crate::style::{configure_fonts, configure_style};

/// Startup system: creates the egui context, winit state, wgpu renderer,
/// and configures fonts and dock layout.
pub(crate) fn editor_startup_system(resources: &mut Resources) {
    let gpu = resources
        .get::<GpuContext>()
        .expect("GpuContext not found — add WindowPlugin before EditorPlugin");
    let window_handle = resources
        .get::<ome_window::WindowHandle>()
        .expect("WindowHandle not found — add WindowPlugin before EditorPlugin");
    let window = window_handle.window();

    let ctx = egui::Context::default();
    configure_fonts(&ctx);
    configure_style(&ctx);

    let winit_state = Arc::new(std::sync::Mutex::new(egui_winit::State::new(
        ctx.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        Some(window.scale_factor() as f32),
        None,
        None,
    )));

    let renderer = egui_wgpu::Renderer::new(gpu.device(), gpu.format(), None, 1, false);

    let overlay = EditorOverlay {
        ctx,
        winit_state: Arc::clone(&winit_state),
        renderer,
        dock_state: crate::state::default_dock_state(),
        selected_entities: Vec::new(),
        last_clicked_index: None,
    };

    let handler: Box<dyn RawEventHandler> = Box::new(EguiEventHandler { winit_state });
    resources.insert(overlay);
    resources.insert(handler);

    tracing::info!("Editor overlay initialized");
}
