//! [`RenderPlugin`] — full game render pipeline targeting the surface.
//!
//! Mirrors `ome_editor_core::viewport::render::render_viewport` but writes
//! to the swapchain surface instead of an offscreen texture. Two passes
//! share one encoder per frame:
//!
//! 1. **Sky** (when an active `SkyRenderer` entity exists) — clears color +
//!    depth, draws procedural gradient + volumetric clouds.
//! 2. **Mesh** — depth-tested rasterization of `MeshRenderer +
//!    GlobalTransform` entities.
//!
//! Used by play-mode binaries via `oh_my_engine::DefaultPlugins`.
//!
//! Post-pivot 2026-05-02: SDF raymarch pass removed. Engine is mesh-only;
//! voxel + DC pipeline (Phase 2.5) will feed mesh chunks into pass 2.

mod path;

use glam::Vec4;
use ome_core::app::App;
use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::stage::Stage;

use crate::VIEWPORT_DEPTH_FORMAT;
use crate::fps::FpsTracker;
use crate::mesh::MeshPassRenderer;
use crate::meshlet::{MeshletBlit, MeshletRenderStage, MeshletRenderStageConfig};
use crate::sky::SkyRenderPass;

use path::{acquire_and_render, RenderPath, SurfaceOutcome};

/// Fallback clear color when no `SkyRenderer` entity is active. Matches the
/// `SkyRenderer` component's default bottom gradient so play and edit modes
/// look identical out of the box.
pub(super) const SKY_FALLBACK: Vec4 = Vec4::new(0.1, 0.2, 0.4, 1.0);

/// Runtime toggle that decides whether [`RenderPlugin`] runs the legacy
/// `MeshPassRenderer` path or the new meshlet GPU-driven pipeline.
///
/// Default is `enabled = false` — the meshlet path is opt-in until
/// Phase 1.E.4 visual validation in the editor. When `enabled = true`,
/// the plugin still runs sky + clear, but replaces the mesh pass with
/// `MeshletRenderStage::render_with_assets` followed by a
/// [`MeshletBlit`] composite onto the surface.
///
/// Toggle at runtime by mutating the resource:
/// `resources.get_mut::<UseMeshletPath>().unwrap().enabled = true;`
#[derive(Debug, Clone, Copy, Default)]
pub struct UseMeshletPath {
    pub enabled: bool,
}

/// Plugin that installs the full render pipeline.
///
/// Inserts [`MeshPassRenderer`], [`SkyRenderPass`] and a surface-sized
/// depth texture as resources at `Stage::Startup`, and runs the per-frame
/// orchestrator at `Stage::Render`.
#[derive(Default)]
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(FpsTracker::new());
        app.insert_resource(UseMeshletPath::default());
        app.add_system(Stage::Startup, init_renderers);
        app.add_system(Stage::Render, render_frame_system);
    }

    fn name(&self) -> &str {
        "RenderPlugin"
    }
}

/// Surface-sized depth texture owned by the render plugin. Recreated when
/// the swapchain size changes.
struct GameDepth {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: (u32, u32),
}

impl GameDepth {
    fn new(device: &wgpu::Device, size: (u32, u32)) -> Self {
        let (texture, view) = create_depth(device, size);
        Self {
            _texture: texture,
            view,
            size,
        }
    }

    fn ensure(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if size == self.size {
            return;
        }
        let (texture, view) = create_depth(device, size);
        self._texture = texture;
        self.view = view;
        self.size = size;
    }
}

fn create_depth(device: &wgpu::Device, size: (u32, u32)) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("game_depth_texture"),
        size: wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VIEWPORT_DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn init_renderers(resources: &mut Resources) {
    if resources.get::<MeshPassRenderer>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        tracing::warn!("RenderPlugin: GpuContext missing at Startup, deferring init");
        return;
    };
    let pipeline_cache = gpu.pipeline_cache();
    let mesh_pass = MeshPassRenderer::new(gpu.device(), gpu.format(), pipeline_cache);
    let sky_pass = SkyRenderPass::new(gpu.device(), gpu.format(), pipeline_cache);
    let depth = GameDepth::new(gpu.device(), gpu.size());
    let meshlet_stage = MeshletRenderStage::new(
        gpu.device(),
        MeshletRenderStageConfig {
            size: gpu.size(),
            ..Default::default()
        },
    );
    let meshlet_blit = MeshletBlit::new(gpu.device(), gpu.format());
    resources.insert(mesh_pass);
    resources.insert(sky_pass);
    resources.insert(depth);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    tracing::info!("RenderPlugin: renderers initialized (legacy + meshlet)");
}

fn render_frame_system(resources: &mut Resources) {
    if let Some(tracker) = resources.get_mut::<FpsTracker>()
        && let Some(fps) = tracker.tick()
    {
        tracing::debug!(fps = format!("{fps:.1}"), "FPS");
    }

    if resources.get::<MeshPassRenderer>().is_none() {
        init_renderers(resources);
    }

    let use_meshlet = resources
        .get::<UseMeshletPath>()
        .map(|t| t.enabled)
        .unwrap_or(false);

    let Some(mut mesh_pass) = resources.remove::<MeshPassRenderer>() else {
        return;
    };
    let Some(mut sky_pass) = resources.remove::<SkyRenderPass>() else {
        resources.insert(mesh_pass);
        return;
    };
    let Some(gpu) = resources.remove::<GpuContext>() else {
        resources.insert(mesh_pass);
        resources.insert(sky_pass);
        return;
    };
    let mut depth = resources
        .remove::<GameDepth>()
        .unwrap_or_else(|| GameDepth::new(gpu.device(), gpu.size()));

    let mut meshlet_stage = resources.remove::<MeshletRenderStage>();
    let meshlet_blit = resources.remove::<MeshletBlit>();

    if use_meshlet {
        if let Some(stage) = meshlet_stage.as_mut() {
            stage.sync_assets_to_gpu(gpu.device(), resources);
        }
    }

    let (w, h) = gpu.size();
    depth.ensure(gpu.device(), (w, h));
    let aspect = w as f32 / h.max(1) as f32;

    let path = if use_meshlet {
        match (meshlet_stage.as_ref(), meshlet_blit.as_ref()) {
            (Some(stage), Some(blit)) => RenderPath::Meshlet { stage, blit },
            _ => RenderPath::Legacy,
        }
    } else {
        RenderPath::Legacy
    };

    let outcome = acquire_and_render(
        &gpu,
        &mut sky_pass,
        &mut mesh_pass,
        &depth.view,
        resources,
        aspect,
        path,
    );

    resources.insert(gpu);
    resources.insert(mesh_pass);
    resources.insert(sky_pass);
    resources.insert(depth);
    if let Some(stage) = meshlet_stage {
        resources.insert(stage);
    }
    if let Some(blit) = meshlet_blit {
        resources.insert(blit);
    }

    match outcome {
        SurfaceOutcome::Presented | SurfaceOutcome::Skip => {}
        SurfaceOutcome::NeedsReconfigure => {
            if let Some(gpu) = resources.get_mut::<GpuContext>() {
                let (w, h) = gpu.size();
                tracing::warn!("Surface outdated, reconfiguring ({w}x{h})");
                gpu.resize(w, h);
            }
        }
        SurfaceOutcome::Error => {
            tracing::error!("Surface validation error — requesting exit");
            if let Some(events) = resources.get_mut::<Events<AppExit>>() {
                events.send(AppExit);
            }
        }
    }
}


