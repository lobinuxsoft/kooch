//! Render systems executed each frame.

use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use wgpu::SurfaceError;

use crate::clear_color::ClearColor;
use crate::fps::FpsTracker;

/// Clears the screen with [`ClearColor`] and presents the frame.
pub fn render_system(resources: &mut Resources) {
    // Tick FPS tracker.
    if let Some(tracker) = resources.get_mut::<FpsTracker>()
        && let Some(fps) = tracker.tick()
    {
        tracing::debug!(fps = format!("{fps:.1}"), "FPS");
    }

    // Copy ClearColor (it's Copy, releases the borrow).
    let color = resources
        .get::<ClearColor>()
        .copied()
        .unwrap_or_default();

    // Scope the immutable borrow of GpuContext so we can mutably borrow
    // resources again in the error path.
    let result = {
        let Some(gpu) = resources.get::<GpuContext>() else {
            return;
        };
        render_frame(gpu, color)
    };

    if let Err(err) = result {
        handle_surface_error(resources, err);
    }
}

/// Acquires a frame, runs a clear-color render pass, and presents.
fn render_frame(gpu: &GpuContext, color: ClearColor) -> Result<(), SurfaceError> {
    let frame = gpu.surface().get_current_texture()?;
    let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("clear_color_encoder"),
        });

    // The render pass clears the attachment on load — no draw calls needed.
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clear_color_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color.to_wgpu()),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    gpu.queue().submit(std::iter::once(encoder.finish()));
    frame.present();

    Ok(())
}

/// Reacts to surface errors: reconfigure on Lost/Outdated, exit on OOM.
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
