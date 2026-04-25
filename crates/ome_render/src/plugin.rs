//! [`RenderPlugin`] — full game render pipeline targeting the surface.
//!
//! Mirrors `ome_editor_core::viewport::render::render_viewport` but writes
//! to the swapchain surface instead of an offscreen texture. Three passes
//! share one encoder per frame:
//!
//! 1. **Sky** (when an active `SkyRenderer` entity exists) — clears color +
//!    depth, draws procedural gradient + volumetric clouds.
//! 2. **Ray-march** — sphere-traces visible SDF entities. Loads the sky
//!    output if it ran first, otherwise clears with internal gradient.
//! 3. **Mesh** — depth-tested rasterization of `MeshRenderer +
//!    GlobalTransform` entities.
//!
//! Used by play-mode binaries via `oh_my_engine::DefaultPlugins`. The
//! `raymarch_demo` example uses the simpler [`crate::RayMarchPlugin`].

use glam::Vec4;
use ome_core::app::App;
use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::stage::Stage;
use ome_core::time::Time;
use ome_ecs::SdfSphere;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::Query;
use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

use crate::VIEWPORT_DEPTH_FORMAT;
use crate::fps::FpsTracker;
use crate::mesh::MeshPassRenderer;
use crate::raymarch::RayMarchRenderer;
use crate::sky::SkyRenderPass;

/// Fallback sky gradient when no `SkyRenderer` entity is active.
/// Matches the defaults used by the editor viewport and the `SkyRenderer`
/// component so play and edit modes look identical out of the box.
const SKY_TOP: Vec4 = Vec4::new(0.5, 0.7, 1.0, 1.0);
const SKY_BOTTOM: Vec4 = Vec4::new(0.1, 0.2, 0.4, 1.0);

/// Plugin that installs the full render pipeline.
///
/// Inserts [`RayMarchRenderer`], [`MeshPassRenderer`], [`SkyRenderPass`]
/// and a surface-sized depth texture as resources at `Stage::Startup`,
/// and runs the per-frame orchestrator at `Stage::Render`.
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
    if resources.get::<RayMarchRenderer>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        tracing::warn!("RenderPlugin: GpuContext missing at Startup, deferring init");
        return;
    };
    let pipeline_cache = gpu.pipeline_cache();
    let raymarch = RayMarchRenderer::new(gpu.device(), gpu.format(), pipeline_cache);
    let mesh_pass = MeshPassRenderer::new(gpu.device(), gpu.format(), pipeline_cache);
    let sky_pass = SkyRenderPass::new(gpu.device(), gpu.format(), pipeline_cache);
    let depth = GameDepth::new(gpu.device(), gpu.size());
    resources.insert(raymarch);
    resources.insert(mesh_pass);
    resources.insert(sky_pass);
    resources.insert(depth);
    tracing::info!("RenderPlugin: renderers initialized");
}

fn render_frame_system(resources: &mut Resources) {
    if let Some(tracker) = resources.get_mut::<FpsTracker>()
        && let Some(fps) = tracker.tick()
    {
        tracing::debug!(fps = format!("{fps:.1}"), "FPS");
    }

    if resources.get::<RayMarchRenderer>().is_none() {
        init_renderers(resources);
    }

    let Some(mut raymarch) = resources.remove::<RayMarchRenderer>() else {
        return;
    };
    let Some(mut mesh_pass) = resources.remove::<MeshPassRenderer>() else {
        resources.insert(raymarch);
        return;
    };
    let Some(mut sky_pass) = resources.remove::<SkyRenderPass>() else {
        resources.insert(raymarch);
        resources.insert(mesh_pass);
        return;
    };
    let Some(gpu) = resources.remove::<GpuContext>() else {
        resources.insert(raymarch);
        resources.insert(mesh_pass);
        resources.insert(sky_pass);
        return;
    };
    let mut depth = resources
        .remove::<GameDepth>()
        .unwrap_or_else(|| GameDepth::new(gpu.device(), gpu.size()));

    let (w, h) = gpu.size();
    depth.ensure(gpu.device(), (w, h));
    let aspect = w as f32 / h.max(1) as f32;

    let outcome = acquire_and_render(
        &gpu,
        &mut sky_pass,
        &mut raymarch,
        &mut mesh_pass,
        &depth.view,
        resources,
        aspect,
    );

    resources.insert(gpu);
    resources.insert(raymarch);
    resources.insert(mesh_pass);
    resources.insert(sky_pass);
    resources.insert(depth);

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

fn acquire_and_render(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    raymarch: &mut RayMarchRenderer,
    mesh_pass: &mut MeshPassRenderer,
    depth_view: &wgpu::TextureView,
    resources: &Resources,
    aspect: f32,
) -> SurfaceOutcome {
    match gpu.surface().get_current_texture() {
        CurrentSurfaceTexture::Success(tex) | CurrentSurfaceTexture::Suboptimal(tex) => {
            render_passes(
                gpu, sky_pass, raymarch, mesh_pass, depth_view, resources, aspect, tex,
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

fn render_passes(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    raymarch: &mut RayMarchRenderer,
    mesh_pass: &mut MeshPassRenderer,
    depth_view: &wgpu::TextureView,
    resources: &Resources,
    aspect: f32,
    frame: SurfaceTexture,
) {
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("game_render_encoder"),
        });

    // Pass 1: Sky.
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

    // Pass 2: Ray-march.
    let has_sdf = has_visible_sdf(resources);
    let camera_ok =
        has_sdf && raymarch.update_camera(gpu.device(), gpu.queue(), resources, aspect);
    if camera_ok {
        raymarch.update_scene(
            gpu.device(),
            gpu.queue(),
            resources,
            SKY_TOP,
            SKY_BOTTOM,
            sky_drawn,
        );
        raymarch.render(&mut encoder, &view, depth_view, !sky_drawn);
    } else if !sky_drawn {
        clear_to_black(&mut encoder, &view, depth_view);
    }

    // Pass 3: Mesh.
    if has_visible_mesh(resources) {
        mesh_pass.render(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            &view,
            depth_view,
            resources,
            aspect,
        );
    }

    gpu.queue().submit(Some(encoder.finish()));
    frame.present();
}

fn clear_to_black(
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
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view: depth,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(1.0),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

fn has_visible_sdf(resources: &Resources) -> bool {
    let query = Query::<&SdfSphere>::new(resources);
    let mut found = false;
    query.for_each(|sphere| {
        if sphere.visible {
            found = true;
        }
    });
    found
}

fn has_visible_mesh(resources: &Resources) -> bool {
    let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
    let mut found = false;
    query.for_each(|(mr, _)| {
        if mr.visible && !mr.mesh.is_empty() {
            found = true;
        }
    });
    found
}
