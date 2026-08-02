//! Editor startup system — initializes egui, winit integration and wgpu renderer.

use std::sync::Arc;

use kooch_core::gpu::GpuContext;
use kooch_core::power::{self, PowerProfile};
use kooch_core::raw_event::RawEventHandler;
use kooch_core::resource::Resources;
use kooch_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use kooch_render::SkyRenderPass;
use kooch_render::Vbuf64Support;
use kooch_render::meshlet::{
    MeshletBlit, MeshletDebugCaps, MeshletDebugMode, MeshletLodSettings, MeshletRenderStage,
    MeshletRenderStageConfig,
};

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
        .get::<kooch_window::WindowHandle>()
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

    let mut renderer = egui_wgpu::Renderer::new(
        gpu.device(),
        gpu.format(),
        egui_wgpu::RendererOptions::default(),
    );

    let pipeline_cache = gpu.pipeline_cache();
    let sky_pass = SkyRenderPass::new(gpu.device(), gpu.format(), pipeline_cache);
    let gizmo_renderer = GizmoRenderer::new(gpu.device(), gpu.format(), pipeline_cache);
    let mesh_gizmo_renderer = MeshGizmoRenderer::new(gpu.device(), gpu.format(), pipeline_cache);
    let viewport = ViewportTarget::new(
        gpu.device(),
        &mut renderer,
        gpu.format(),
        INITIAL_VIEWPORT_SIZE,
    );

    let vbuf64 = Vbuf64Support::detect(gpu.device());
    let debug_caps = MeshletDebugCaps::detect(gpu.device());
    let mut meshlet_stage = MeshletRenderStage::new(
        gpu.device(),
        MeshletRenderStageConfig {
            size: INITIAL_VIEWPORT_SIZE,
            vbuf64,
            debug_caps,
            ..Default::default()
        },
    );
    // #463.4 — opt the editor in to GPU timestamp queries. No-op
    // when the adapter does not expose `Features::TIMESTAMP_QUERY`;
    // the perf HUD then reports "GPU: n/a".
    meshlet_stage.enable_gpu_timers(gpu.device(), gpu.queue(), gpu.adapter());
    // #463.5 — share the engine VRAM tracker so the meshlet stage
    // bumps the counter on every persistent allocation it owns
    // (pool register, render-target resize, …). The Arc is also
    // inserted as a Resource further down so the perf HUD's
    // update path reads from the same shared counter.
    let vram_tracker = std::sync::Arc::new(kooch_render::EngineVramTracker::new());
    meshlet_stage.set_vram_tracker(vram_tracker.clone());
    let meshlet_blit = MeshletBlit::new(gpu.device(), gpu.format());

    let overlay = EditorOverlay {
        focused_tab: None,
        asset_nav: Default::default(),
        inspector_nav: Default::default(),
        ctx,
        winit_state: Arc::clone(&winit_state),
        renderer,
        dock_state: crate::state::default_dock_state(),
        selected_entities: Vec::new(),
        pinned_gizmos: std::collections::HashSet::new(),
        last_clicked_index: None,
        rotation_euler_cache: std::collections::HashMap::new(),
        rotation_display_mode: crate::state::RotationDisplayMode::default(),
        snap_settings: kooch_gizmos_handles::SnapSettings::default(),
        gizmo_drag_start: None,
        selected_asset: None,
        current_folder: None,
    };

    let handler: Box<dyn RawEventHandler> = Box::new(EguiEventHandler { winit_state });
    let power_profile: PowerProfile = power::detect();
    resources.insert(overlay);
    // First in the list on purpose: a key typed into a focused text
    // field belongs to egui, and gameplay input must not also see it.
    resources
        .get_or_default::<kooch_core::raw_event::RawEventHandlers>()
        .push(handler);
    resources.insert(sky_pass);
    resources.insert(gizmo_renderer);
    resources.insert(mesh_gizmo_renderer);
    resources.insert(GizmoBatch::default());
    resources.insert(MeshBatch::default());
    resources.insert(viewport);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    resources.insert(vram_tracker);
    // Debug-view selector for the meshlet pipeline (#451). Default
    // Off keeps the production normal-debug path; the View toolbar
    // dropdown writes through this resource per-frame.
    resources.insert(MeshletDebugMode::default());
    // Capability probe for the advanced debug modes (#454). The
    // dropdown filter consults this so a device missing
    // `TEXTURE_ATOMIC` never lists a mode whose pipeline would fail
    // validation.
    resources.insert(debug_caps);
    // Continuous-LOD threshold (#462). Default 1.0 px is the
    // production target; the View toolbar exposes a slider so
    // artists can crank it higher to force coarser LOD selection
    // at editor distances and visually sanity-check the chain.
    resources.insert(MeshletLodSettings::default());
    resources.insert(power_profile);

    tracing::info!("Editor overlay initialized");
}
