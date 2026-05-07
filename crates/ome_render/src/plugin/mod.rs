//! Engine render plugins.
//!
//! - [`RenderPlugin`] — full game render pipeline targeting the surface
//!   (sky + meshlet stage + blit). Used by play-mode binaries via
//!   `oh_my_engine::DefaultPlugins`.
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
use ome_core::app::App;
use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::stage::Stage;
use ome_core::time::Time;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::perspective_camera::PerspectiveCamera;
use ome_ecs::query::Query;
use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

use crate::VIEWPORT_DEPTH_FORMAT;
use crate::fps::FpsTracker;
use crate::meshlet::{MeshletBlit, MeshletRenderStage, MeshletRenderStageConfig};
use crate::sky::SkyRenderPass;

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
        app.insert_resource(FpsTracker::new());
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
    resources.insert(sky_pass);
    resources.insert(depth);
    resources.insert(meshlet_stage);
    resources.insert(meshlet_blit);
    tracing::info!("RenderPlugin: renderers initialized (sky + meshlet)");
}

fn render_frame_system(resources: &mut Resources) {
    if let Some(tracker) = resources.get_mut::<FpsTracker>()
        && let Some(fps) = tracker.tick()
    {
        tracing::debug!(fps = format!("{fps:.1}"), "FPS");
    }

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
    meshlet_stage.sync_assets_to_gpu(gpu.device(), resources);

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
    let (view_proj, cam_pos) = active_camera_matrices(resources, aspect)
        .unwrap_or((glam::Mat4::IDENTITY, glam::Vec3::ZERO));
    let _ =
        meshlet_stage.render_with_assets(gpu.device(), gpu.queue(), resources, view_proj, cam_pos);

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("game_render_encoder"),
        });

    let sky_drawn = if let Some(active_sky) = SkyRenderPass::active_sky(resources) {
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

    // Composite the meshlet stage's color over the sky only when the
    // stage has GPU-resident meshes. Without this guard the blit would
    // copy the stage's empty color buffer over the sky every frame,
    // blanking the surface to black until something is registered.
    if meshlet_stage.gpu_mesh_count() > 0 {
        meshlet_blit.blit(gpu.device(), &mut encoder, meshlet_stage.color_view(), &view);
    }

    gpu.queue().submit(Some(encoder.finish()));
    frame.present();
}

fn active_camera_matrices(
    resources: &Resources,
    aspect: f32,
) -> Option<(glam::Mat4, glam::Vec3)> {
    // Highest-priority active `PerspectiveCamera` wins. Game runtime
    // ties the same way the editor does: priority is the contract,
    // not iteration order.
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, glam::Mat4, glam::Vec3)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        if let Some((p, _, _)) = best
            && cam.priority <= p
        {
            return;
        }
        let world = gt.matrix;
        let view = world.inverse();
        let fov_y_rad = cam.fov.to_radians().max(1.0_f32.to_radians());
        let proj = crate::projection::perspective_rh_reverse_z(
            fov_y_rad,
            aspect.max(0.01),
            cam.near.max(0.001),
            cam.far.max(cam.near + 0.001),
        );
        let cam_pos = world.w_axis.truncate();
        best = Some((cam.priority, proj * view, cam_pos));
    });
    best.map(|(_, vp, p)| (vp, p))
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
