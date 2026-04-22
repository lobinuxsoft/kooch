//! Plugin that installs the ray-marching renderer.
//!
//! Reads the active `PerspectiveCamera + Transform` and every visible
//! SDF shape entity (Sphere, Box, Capsule, Cylinder, Torus, Plane) with
//! an optional `SdfBlend` component each frame, uploads them to GPU
//! buffers, and draws a fullscreen pass.

use glam::Vec4;
use ome_core::app::App;
use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::plugin::Plugin;
use ome_core::resource::Resources;
use ome_core::stage::Stage;

use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

use crate::fps::FpsTracker;
use crate::raymarch::RayMarchRenderer;

/// Sky colors used when a ray misses every SDF.
#[derive(Debug, Clone, Copy)]
pub struct SkyGradient {
    /// Color at the zenith (ray.direction.y = 1).
    pub top: Vec4,
    /// Color at the horizon / below (ray.direction.y <= 0).
    pub bottom: Vec4,
}

impl Default for SkyGradient {
    fn default() -> Self {
        Self {
            top: Vec4::new(0.50, 0.70, 1.00, 1.0),
            bottom: Vec4::new(0.10, 0.20, 0.40, 1.0),
        }
    }
}

/// Plugin that renders SDF entities via sphere tracing.
#[derive(Default)]
pub struct RayMarchPlugin {
    pub sky: SkyGradient,
}

impl Plugin for RayMarchPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.sky);
        app.insert_resource(FpsTracker::new());
        app.add_system(Stage::Startup, init_renderer);
        app.add_system(Stage::Render, raymarch_system);
    }

    fn name(&self) -> &str {
        "RayMarchPlugin"
    }
}

/// Creates the [`RayMarchRenderer`] once the [`GpuContext`] is available.
fn init_renderer(resources: &mut Resources) {
    if resources.get::<RayMarchRenderer>().is_some() {
        return;
    }
    let Some(gpu) = resources.get::<GpuContext>() else {
        tracing::warn!("RayMarchPlugin: GpuContext missing at Startup, renderer not created");
        return;
    };
    let renderer = RayMarchRenderer::new(gpu.device(), gpu.format());
    resources.insert(renderer);
    tracing::info!("RayMarchPlugin: renderer initialized");
}

fn raymarch_system(resources: &mut Resources) {
    if let Some(tracker) = resources.get_mut::<FpsTracker>()
        && let Some(fps) = tracker.tick()
    {
        tracing::debug!(fps = format!("{fps:.1}"), "FPS");
    }

    // Lazy init in case GpuContext wasn't ready at Startup.
    if resources.get::<RayMarchRenderer>().is_none() {
        init_renderer(resources);
    }

    let sky = resources.get::<SkyGradient>().copied().unwrap_or_default();

    let Some(mut renderer) = resources.remove::<RayMarchRenderer>() else {
        return;
    };
    let Some(gpu) = resources.remove::<GpuContext>() else {
        resources.insert(renderer);
        return;
    };

    let (w, h) = gpu.size();
    let aspect = w as f32 / h.max(1) as f32;
    let has_camera = renderer.update_camera(gpu.device(), gpu.queue(), resources, aspect);
    renderer.update_scene(gpu.device(), gpu.queue(), resources, sky.top, sky.bottom);

    let outcome = if has_camera {
        acquire_and_render(&gpu, &renderer)
    } else {
        SurfaceOutcome::Skip
    };

    resources.insert(gpu);
    resources.insert(renderer);

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

fn acquire_and_render(gpu: &GpuContext, renderer: &RayMarchRenderer) -> SurfaceOutcome {
    match gpu.surface().get_current_texture() {
        CurrentSurfaceTexture::Success(tex) | CurrentSurfaceTexture::Suboptimal(tex) => {
            render_frame(gpu, renderer, tex);
            SurfaceOutcome::Presented
        }
        CurrentSurfaceTexture::Outdated | CurrentSurfaceTexture::Lost => {
            SurfaceOutcome::NeedsReconfigure
        }
        CurrentSurfaceTexture::Occluded | CurrentSurfaceTexture::Timeout => {
            SurfaceOutcome::Skip
        }
        CurrentSurfaceTexture::Validation => SurfaceOutcome::Error,
    }
}

fn render_frame(gpu: &GpuContext, renderer: &RayMarchRenderer, frame: SurfaceTexture) {
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("raymarch_encoder"),
        });
    renderer.render(&mut encoder, &view);
    gpu.queue().submit(std::iter::once(encoder.finish()));
    frame.present();
}
