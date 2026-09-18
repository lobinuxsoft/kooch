//! Putting each torn-off panel's pass on its own window.
//!
//! 🔴 One window at a time, each submitted before the next is prepared: the egui renderer keeps one
//! vertex buffer and one screen uniform, and a second window's upload would land on the first
//! window's draw.

use kooch_core::gpu::GpuContext;

use super::{Live, SharedLive};

/// Uploads the textures the nested passes created. Before the main window draws: whichever pass
/// ends first carries a font atlas update, and the main one draws with it too.
pub(crate) fn apply_textures(
    renderer: &mut egui_wgpu::Renderer,
    gpu: &GpuContext,
    live: &SharedLive,
) {
    for open in live.lock().unwrap().iter() {
        if let Some(output) = &open.output {
            for (id, delta) in &output.textures_delta.set {
                renderer.update_texture(gpu.device(), gpu.queue(), *id, delta);
            }
        }
    }
}

/// Draws and presents every window with a pass waiting. A window that cannot take a frame is
/// skipped; the main window's frame goes on.
pub(crate) fn paint_all(renderer: &mut egui_wgpu::Renderer, gpu: &GpuContext, live: &SharedLive) {
    let mut list = live.lock().unwrap();
    for open in list.iter_mut() {
        let Some(output) = open.output.take() else {
            continue;
        };
        open.state
            .handle_platform_output(&open.window, output.platform_output.clone());
        paint(renderer, gpu, open, &output);
        for id in &output.textures_delta.free {
            renderer.free_texture(id);
        }
    }
}

fn paint(
    renderer: &mut egui_wgpu::Renderer,
    gpu: &GpuContext,
    open: &mut Live,
    output: &egui::FullOutput,
) {
    let size = open.window.inner_size();
    if size.width == 0 || size.height == 0 {
        return;
    }
    if (size.width, size.height) != (open.config.width, open.config.height) {
        open.config.width = size.width;
        open.config.height = size.height;
        open.surface.configure(gpu.device(), &open.config);
    }
    let frame = match open.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(tex)
        | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
        status => {
            tracing::debug!(?status, tab = ?open.tab, "panel window skipped a frame");
            return;
        }
    };
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [size.width, size.height],
        pixels_per_point: output.pixels_per_point,
    };
    let primitives = open
        .state
        .egui_ctx()
        .tessellate(output.shapes.clone(), output.pixels_per_point);
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("panel_window_encoder"),
        });
    let mut buffers = renderer.update_buffers(
        gpu.device(),
        gpu.queue(),
        &mut encoder,
        &primitives,
        &screen,
    );
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("panel_window_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.1,
                        g: 0.1,
                        b: 0.1,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let mut pass = pass.forget_lifetime();
        renderer.render(&mut pass, &primitives, &screen);
    }
    buffers.push(encoder.finish());
    gpu.queue().submit(buffers);
    frame.present();
}
