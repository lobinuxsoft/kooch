//! Editor startup system — initializes egui, winit integration and wgpu renderer.

use std::sync::Arc;

use ome_core::gpu::GpuContext;
use ome_core::power::{self, PowerProfile};
use ome_core::raw_event::RawEventHandler;
use ome_core::resource::Resources;
use ome_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use ome_render::meshlet::{
    MeshletBlit, MeshletDebugMode, MeshletLodSettings, MeshletRenderStage,
    MeshletRenderStageConfig,
};
use ome_render::SkyRenderPass;
use ome_world::{ChunkManager, ProceduralCitySource};

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

    let mut meshlet_stage = MeshletRenderStage::new(
        gpu.device(),
        MeshletRenderStageConfig {
            size: INITIAL_VIEWPORT_SIZE,
            ..Default::default()
        },
    );
    // #463.4 — opt the editor in to GPU timestamp queries. No-op
    // when the adapter does not expose `Features::TIMESTAMP_QUERY`;
    // the perf HUD then reports "GPU: n/a".
    meshlet_stage.enable_gpu_timers(gpu.device(), gpu.queue(), gpu.adapter());
    let meshlet_blit = MeshletBlit::new(gpu.device(), gpu.format());

    let overlay = EditorOverlay {
        ctx,
        winit_state: Arc::clone(&winit_state),
        renderer,
        dock_state: crate::state::default_dock_state(),
        selected_entities: Vec::new(),
        last_clicked_index: None,
        rotation_euler_cache: std::collections::HashMap::new(),
        rotation_display_mode: crate::state::RotationDisplayMode::default(),
        snap_settings: ome_gizmos_handles::SnapSettings::default(),
        gizmo_drag_start: None,
    };

    // Wire the procedural city as the editor's default content source —
    // makes streamed chunks visible the moment a `StreamingFocus` lands
    // on the camera (#363). Game runtime / headless tests opt in
    // explicitly via their own `register_content_source` call.
    if let Some(manager) = resources.get_mut::<ChunkManager>() {
        manager.register_content_source(Box::new(ProceduralCitySource::default()));
        tracing::info!(
            target: "ome_editor_core::systems::startup",
            "ProceduralCitySource registered as default content source",
        );
    } else {
        tracing::warn!(
            "ChunkManager resource missing — ProceduralCitySource not registered. \
             Add WorldStreamingPlugin before EditorPlugin."
        );
    }

    let handler: Box<dyn RawEventHandler> = Box::new(EguiEventHandler { winit_state });
    let power_profile: PowerProfile = power::detect();
    resources.insert(overlay);
    resources.insert(handler);
    resources.insert(sky_pass);
    resources.insert(gizmo_renderer);
    resources.insert(mesh_gizmo_renderer);
    resources.insert(GizmoBatch::default());
    resources.insert(MeshBatch::default());
    resources.insert(viewport);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    // Debug-view selector for the meshlet pipeline (#451). Default
    // Off keeps the production normal-debug path; the View toolbar
    // dropdown writes through this resource per-frame.
    resources.insert(MeshletDebugMode::default());
    // Continuous-LOD threshold (#462). Default 1.0 px is the
    // production target; the View toolbar exposes a slider so
    // artists can crank it higher to force coarser LOD selection
    // at editor distances and visually sanity-check the chain.
    resources.insert(MeshletLodSettings::default());
    resources.insert(power_profile);

    tracing::info!("Editor overlay initialized");
}
