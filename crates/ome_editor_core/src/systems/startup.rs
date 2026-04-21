//! Editor startup system — initializes egui, winit integration and wgpu renderer.

use std::sync::Arc;

use ome_core::gpu::GpuContext;
use ome_core::raw_event::RawEventHandler;
use ome_core::resource::Resources;
use ome_render::RayMarchRenderer;

use crate::state::{EditorOverlay, EguiEventHandler};
use crate::style::{configure_fonts, configure_style};
use crate::viewport::ViewportTarget;

/// Initial backing texture size for the viewport. Overwritten by the first
/// layout pass of the View panel.
const INITIAL_VIEWPORT_SIZE: (u32, u32) = (512, 512);

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

    let mut renderer = egui_wgpu::Renderer::new(gpu.device(), gpu.format(), None, 1, false);

    let raymarch = RayMarchRenderer::new(gpu.device(), gpu.format());
    let viewport = ViewportTarget::new(
        gpu.device(),
        &mut renderer,
        gpu.format(),
        INITIAL_VIEWPORT_SIZE,
    );

    let overlay = EditorOverlay {
        ctx,
        winit_state: Arc::clone(&winit_state),
        renderer,
        dock_state: crate::state::default_dock_state(),
        selected_entities: Vec::new(),
        last_clicked_index: None,
        rotation_euler_cache: std::collections::HashMap::new(),
        rotation_display_mode: crate::state::RotationDisplayMode::default(),
    };

    let handler: Box<dyn RawEventHandler> = Box::new(EguiEventHandler { winit_state });
    resources.insert(overlay);
    resources.insert(handler);
    resources.insert(raymarch);
    resources.insert(viewport);

    tracing::info!("Editor overlay initialized");
}
