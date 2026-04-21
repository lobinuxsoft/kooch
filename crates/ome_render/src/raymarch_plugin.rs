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
use wgpu::SurfaceError;

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

    let result = if has_camera {
        render_frame(&gpu, &renderer)
    } else {
        Ok(())
    };

    resources.insert(gpu);
    resources.insert(renderer);

    if let Err(err) = result {
        handle_surface_error(resources, err);
    }
}

fn render_frame(gpu: &GpuContext, renderer: &RayMarchRenderer) -> Result<(), SurfaceError> {
    let frame = gpu.surface().get_current_texture()?;
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
    Ok(())
}

fn handle_surface_error(resources: &mut Resources, err: SurfaceError) {
    match err {
        SurfaceError::Outdated | SurfaceError::Lost => {
            if let Some(gpu) = resources.get_mut::<GpuContext>() {
                let (w, h) = gpu.size();
                tracing::warn!(%err, "Surface lost, reconfiguring ({w}x{h})");
                gpu.resize(w, h);
            }
        }
        SurfaceError::OutOfMemory => {
            tracing::error!("GPU out of memory — requesting exit");
            if let Some(events) = resources.get_mut::<Events<AppExit>>() {
                events.send(AppExit);
            }
        }
        other => {
            tracing::warn!(%other, "Surface error, skipping frame");
        }
    }
}
