//! Editor startup system — initializes egui, winit integration and wgpu renderer.

use std::sync::Arc;

use kooch_core::gpu::GpuContext;
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
    // bumps the counter on every persistent allocation it owns (pool register, render-target
    // resize, …). The Arc is also inserted as a Resource further down so the perf HUD's update path
    // reads from the same shared counter.
    let vram_tracker = std::sync::Arc::new(kooch_render::EngineVramTracker::new());
    meshlet_stage.set_vram_tracker(vram_tracker.clone());
    let meshlet_blit = MeshletBlit::new(
        gpu.device(),
        gpu.format(),
        kooch_render::VIEWPORT_DEPTH_FORMAT,
    );
    // #785 — per-pass GPU timings for the editor. Built here, while the
    // `gpu` borrow is alive, and inserted below with the rest.
    let gpu_scopes = kooch_core::gpu::GpuScopes::new(gpu.device(), gpu.queue());

    // The Game panel's view. A second view of this stage rather than a
    // second stage: it shares the mesh pool, the scene instances and the
    // cull pipelines, and owns only what depends on where its camera is.
    let game_view = crate::viewport::GameView::new(
        gpu.device(),
        &mut renderer,
        gpu.format(),
        INITIAL_VIEWPORT_SIZE,
        &mut meshlet_stage,
    );

    // The Shader Graph's preview (#1159). Not a view of the stage at all: it rasterises one
    // primitive with the graph's own surface and lights it by itself.
    let shader_preview =
        crate::viewport::ShaderPreview::new(gpu.device(), gpu.queue(), &mut renderer, gpu.format());

    let overlay = EditorOverlay {
        element_mode: crate::block_edit::ElementMode::default(),
        shape_drag_start: None,
        focused_tab: None,
        asset_nav: Default::default(),
        inspector_nav: Default::default(),
        ctx,
        winit_state: Arc::clone(&winit_state),
        renderer,
        dock_state: crate::state::default_dock_state(),
        windows: crate::os_windows::OsWindows::default(),
        game_resolution: Default::default(),
        selected_entities: Vec::new(),
        pinned_gizmos: std::collections::HashSet::new(),
        last_clicked_index: None,
        rotation_euler_cache: std::collections::HashMap::new(),
        rotation_display_mode: crate::state::RotationDisplayMode::default(),
        snap_settings: kooch_gizmos_handles::SnapSettings::default(),
        gizmo_drag_start: None,
        selected_asset: None,
        build_selection: None,
        current_folder: None,
    };

    let live = std::sync::Arc::clone(&overlay.windows.live);
    crate::os_windows::install(&overlay.ctx, std::sync::Arc::clone(&live));
    let handler: Box<dyn RawEventHandler> = Box::new(EguiEventHandler {
        winit_state,
        live,
        root: window.id(),
    });
    resources.insert(overlay);
    // Today the only handler: the editor builds its own plugin set in `bootstrap.rs` and
    // `InputPlugin` is not in it.
    resources
        .get_or_default::<kooch_core::raw_event::RawEventHandlers>()
        .push(handler);
    resources.insert(sky_pass);
    resources.insert(gizmo_renderer);
    resources.insert(mesh_gizmo_renderer);
    resources.insert(GizmoBatch::default());
    resources.insert(MeshBatch::default());
    resources.insert(viewport);
    resources.insert(game_view);
    resources.insert(shader_preview);
    // Beside the stage, because the stage's own asset sync is what drains it.
    resources.insert(kooch_render::meshlet::GeneratedMeshes::new());
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    resources.insert(vram_tracker);
    // The stage's own scopes (`shadows`, `cull`, `raster + shade`) are recorded by `kooch_render`
    // the moment this resource exists; without it the editor could profile its CPU and nothing
    // else, which is half the question when the thing being authored is a frame.
    if let Some(scopes) = gpu_scopes {
        resources.insert(scopes);
        tracing::info!("editor: GPU scopes enabled");
    }
    // Debug-view selector for the meshlet pipeline (#451). Default
    // Off keeps the production normal-debug path; the View toolbar
    // dropdown writes through this resource per-frame.
    resources.insert(MeshletDebugMode::default());
    // Capability probe for the advanced debug modes (#454). The dropdown filter consults this so a
    // device missing `TEXTURE_ATOMIC` never lists a mode whose pipeline would fail validation.
    resources.insert(debug_caps);
    // Continuous-LOD threshold (#462). Default 1.0 px is the production target; the View toolbar
    // exposes a slider so artists can crank it higher to force coarser LOD selection at editor
    // distances and visually sanity-check the chain.
    resources.insert(MeshletLodSettings::default());

    tracing::info!("Editor overlay initialized");
}
