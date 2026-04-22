//! Render systems executed each frame.

use ome_core::event::{AppExit, Events};
use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

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
    let outcome = {
        let Some(gpu) = resources.get::<GpuContext>() else {
            return;
        };
        let frame = gpu.surface().get_current_texture();
        match frame {
            CurrentSurfaceTexture::Success(tex) | CurrentSurfaceTexture::Suboptimal(tex) => {
                render_frame(gpu, color, tex);
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
    };

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

/// Runs a clear-color render pass on `frame` and presents it.
fn render_frame(gpu: &GpuContext, color: ClearColor, frame: SurfaceTexture) {
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
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color.to_wgpu()),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }

    gpu.queue().submit(std::iter::once(encoder.finish()));
    frame.present();
}
