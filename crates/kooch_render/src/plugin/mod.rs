//! Engine render plugins.
//!
//! - [`RenderPlugin`] — full game render pipeline targeting the surface
//!   (sky + meshlet stage + blit). Used by play-mode binaries via
//!   `kooch::DefaultPlugins`.
//! - [`AssetPlugin`] (in [`assets`]) — installs `AssetServer`,
//!   `AssetDatabase`, and the `Assets<T>` storages for every asset type
//!   the engine knows how to load. Independent of the GPU pipeline,
//!   so headless tools can install asset loading without rendering.
//!
//! Post-pivot 2026-05-02: SDF raymarch pass removed. Engine is mesh-only;
//! voxel + DC pipeline (Phase 2.5) will feed mesh chunks into pass 2.

pub mod assets;

pub use assets::AssetPlugin;

use glam::Vec4;
use kooch_core::app::App;
use kooch_core::event::{AppExit, Events};
use kooch_core::gpu::GpuContext;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_core::time::Time;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::query::Query;
use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

use crate::VIEWPORT_DEPTH_FORMAT;
use crate::meshlet::{MeshletBlit, MeshletDebugCaps, MeshletRenderStage, MeshletRenderStageConfig};
use crate::sky::SkyRenderPass;
use crate::vbuf64::Vbuf64Support;

/// Fallback clear color when no `SkyRenderer` entity is active. Matches the
/// `SkyRenderer` component's default bottom gradient so play and edit modes
/// look identical out of the box.
const SKY_FALLBACK: Vec4 = Vec4::new(0.1, 0.2, 0.4, 1.0);

/// Plugin that installs the full render pipeline.
///
/// Inserts [`SkyRenderPass`], [`MeshletRenderStage`], [`MeshletBlit`] and a
/// surface-sized depth texture as resources at `Stage::Startup`, and runs
/// the per-frame orchestrator at `Stage::Render`.
#[derive(Default)]
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
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
    if resources.get::<MeshletRenderStage>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        tracing::warn!("RenderPlugin: GpuContext missing at Startup, deferring init");
        return;
    };
    let pipeline_cache = gpu.pipeline_cache();
    let vbuf64 = Vbuf64Support::detect(gpu.device());
    let debug_caps = MeshletDebugCaps::detect(gpu.device());
    let sky_pass = SkyRenderPass::new(gpu.device(), gpu.format(), pipeline_cache);
    let depth = GameDepth::new(gpu.device(), gpu.size());
    let meshlet_stage = MeshletRenderStage::new(
        gpu.device(),
        MeshletRenderStageConfig {
            size: gpu.size(),
            vbuf64,
            debug_caps,
            ..Default::default()
        },
    );
    let mut meshlet_stage = meshlet_stage;
    // The editor did this at its own startup and a game never did, so the
    // GPU frame time existed on this adapter and was reported by nobody.
    // A no-op on adapters without `TIMESTAMP_QUERY`.
    meshlet_stage.enable_gpu_timers(gpu.device(), gpu.queue(), gpu.adapter());

    let meshlet_blit = MeshletBlit::new(gpu.device(), gpu.format());
    // #785 — per-pass GPU timings. `None` in a build without the
    // `gpu-profiler` feature, and the render code below asks for the
    // resource the same way either way.
    let gpu_scopes = kooch_core::gpu::GpuScopes::new(gpu.device(), gpu.queue());
    resources.insert(vbuf64);
    resources.insert(debug_caps);
    resources.insert(sky_pass);
    resources.insert(depth);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    if let Some(gpu_scopes) = gpu_scopes {
        resources.insert(gpu_scopes);
        tracing::info!("RenderPlugin: GPU scopes enabled");
    }
    tracing::info!("RenderPlugin: renderers initialized (sky + meshlet)");
}

fn render_frame_system(resources: &mut Resources) {
    if resources.get::<MeshletRenderStage>().is_none() {
        init_renderers(resources);
    }

    let Some(mut sky_pass) = resources.remove::<SkyRenderPass>() else {
        return;
    };
    let Some(gpu) = resources.remove::<GpuContext>() else {
        resources.insert(sky_pass);
        return;
    };
    let Some(mut meshlet_stage) = resources.remove::<MeshletRenderStage>() else {
        resources.insert(gpu);
        resources.insert(sky_pass);
        return;
    };
    let Some(meshlet_blit) = resources.remove::<MeshletBlit>() else {
        resources.insert(gpu);
        resources.insert(sky_pass);
        resources.insert(meshlet_stage);
        return;
    };
    let mut depth = resources
        .remove::<GameDepth>()
        .unwrap_or_else(|| GameDepth::new(gpu.device(), gpu.size()));

    let (w, h) = gpu.size();
    depth.ensure(gpu.device(), (w, h));
    let aspect = w as f32 / h.max(1) as f32;

    meshlet_stage.resize(gpu.device(), (w, h));
    meshlet_stage.sync_assets_to_gpu(gpu.device(), gpu.queue(), resources);

    let outcome = acquire_and_render(
        &gpu,
        &mut sky_pass,
        &mut meshlet_stage,
        &meshlet_blit,
        &depth.view,
        resources,
        aspect,
    );

    resources.insert(gpu);
    resources.insert(sky_pass);
    resources.insert(depth);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);

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

enum SurfaceOutcome {
    Presented,
    Skip,
    NeedsReconfigure,
    Error,
}

#[allow(clippy::too_many_arguments)]
fn acquire_and_render(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    meshlet_stage: &mut MeshletRenderStage,
    meshlet_blit: &MeshletBlit,
    depth_view: &wgpu::TextureView,
    resources: &mut Resources,
    aspect: f32,
) -> SurfaceOutcome {
    match gpu.surface().get_current_texture() {
        CurrentSurfaceTexture::Success(tex) | CurrentSurfaceTexture::Suboptimal(tex) => {
            render_passes(
                gpu,
                sky_pass,
                meshlet_stage,
                meshlet_blit,
                depth_view,
                resources,
                aspect,
                tex,
            );
            SurfaceOutcome::Presented
        }
        CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
            SurfaceOutcome::NeedsReconfigure
        }
        CurrentSurfaceTexture::Occluded | CurrentSurfaceTexture::Timeout => SurfaceOutcome::Skip,
        CurrentSurfaceTexture::Validation => SurfaceOutcome::Error,
    }
}

#[allow(clippy::too_many_arguments)]
fn render_passes(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    meshlet_stage: &mut MeshletRenderStage,
    meshlet_blit: &MeshletBlit,
    depth_view: &wgpu::TextureView,
    resources: &mut Resources,
    aspect: f32,
    frame: SurfaceTexture,
) {
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    // The meshlet stage submits its own command buffer (cull + raster
    // + deferred) before we record the surface-targeted encoder. Order
    // matters: blit reads the stage's color view, so the stage's submit
    // must complete first on the queue.
    let camera = active_camera(resources);
    // The sky draws only when the scene really has a camera; the meshlet
    // stage falls back to a default lens rather than to an identity
    // matrix, which is not a projection.
    let stats = meshlet_stage.render_with_assets_primary(
        gpu.device(),
        gpu.queue(),
        resources,
        &camera.unwrap_or_default(),
        aspect,
    );

    // The one measurement a game could not otherwise have: the editor
    // reads these stats, and until now a windowed game threw them away.
    // Written into the engine's own metrics rather than kept here, so
    // there is one place that answers "how long did the frame take".
    if let Some(metrics) = resources.get_mut::<kooch_core::frame_metrics::FrameMetrics>() {
        metrics.gpu_frame_ms = stats.gpu_frame_ms;
    }

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("game_render_encoder"),
        });

    // #785 — the sky and the blit are the per-pixel work outside the
    // meshlet stage, and #771 accuses the sky specifically. Timing it
    // here is what turns that accusation into a number.
    let scopes = resources.get::<kooch_core::gpu::GpuScopes>();
    let sky_query = scopes.map(|s| s.begin("sky", &mut encoder));

    let sky_drawn = if let (Some(active_sky), Some(camera)) =
        (SkyRenderPass::active_sky(resources), camera.as_ref())
    {
        let time_secs = resources
            .get::<Time>()
            .map(|t| t.elapsed_secs())
            .unwrap_or(0.0);
        sky_pass.render(
            gpu.queue(),
            &mut encoder,
            &view,
            depth_view,
            resources,
            camera,
            aspect,
            active_sky,
            time_secs,
        )
    } else {
        false
    };

    if !sky_drawn {
        clear_with_gradient(&mut encoder, &view, depth_view);
    }
    if let (Some(scopes), Some(query)) = (scopes, sky_query) {
        scopes.end(&mut encoder, query);
    }

    // Composite the meshlet stage's color over the sky only when the
    // stage has GPU-resident meshes. Without this guard the blit would
    // copy the stage's empty color buffer over the sky every frame,
    // blanking the surface to black until something is registered.
    if meshlet_stage.gpu_mesh_count() > 0 {
        let blit_query = scopes.map(|s| s.begin("blit", &mut encoder));
        meshlet_blit.blit(
            gpu.device(),
            &mut encoder,
            meshlet_stage.color_view(),
            &view,
        );
        if let (Some(scopes), Some(query)) = (scopes, blit_query) {
            scopes.end(&mut encoder, query);
        }
    }

    // The frame's last encoder, so this is where the timestamps are
    // copied out — including the meshlet stage's, which were written
    // into an encoder submitted before this one and are therefore
    // already resolved on the queue by the time this copy runs.
    if let Some(mut scopes) = resources.remove::<kooch_core::gpu::GpuScopes>() {
        scopes.resolve(&mut encoder);
        gpu.queue().submit(Some(encoder.finish()));
        frame.present();
        // After every submit of the frame, never between them: an
        // encoder still holding open queries makes this fail.
        scopes.end_frame(gpu.queue());
        resources.insert(scopes);
        return;
    }

    gpu.queue().submit(Some(encoder.finish()));
    frame.present();
}

fn active_camera(resources: &Resources) -> Option<crate::ViewCamera> {
    // Highest-priority active `PerspectiveCamera` wins. Game runtime
    // ties the same way the editor does: priority is the contract,
    // not iteration order.
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, crate::ViewCamera)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        if let Some((p, _)) = best
            && cam.priority <= p
        {
            return;
        }
        best = Some((cam.priority, crate::ViewCamera::from_components(cam, gt)));
    });
    best.map(|(_, camera)| camera)
}

fn clear_with_gradient(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth: &wgpu::TextureView,
) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("game_clear_pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color {
                    r: SKY_FALLBACK.x as f64,
                    g: SKY_FALLBACK.y as f64,
                    b: SKY_FALLBACK.z as f64,
                    a: SKY_FALLBACK.w as f64,
                }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}
