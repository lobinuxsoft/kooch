//! Egui tessellation + wgpu render pass + surface present.

use egui_wgpu::ScreenDescriptor;
use winit::window::Window;

use kooch_core::gpu::GpuContext;

use crate::state::EditorOverlay;

/// Runs platform output handling, tessellation, and renders the editor UI
/// to the current surface texture.
///
/// Returns `true` if the frame was presented successfully. Returns `false`
/// when the surface texture could not be acquired (caller should skip the
/// frame and keep resources intact).
pub(crate) fn present_editor_frame(
    gpu: &GpuContext,
    overlay: &mut EditorOverlay,
    window: &Window,
    full_output: egui::FullOutput,
    mut scopes: Option<&mut kooch_core::gpu::GpuScopes>,
) -> bool {
    // Handle platform output (cursor icon, clipboard, etc.).
    {
        let mut state = overlay.winit_state.lock().unwrap();
        state.handle_platform_output(window, full_output.platform_output);
    }

    // Tessellate.
    let pixels_per_point = overlay.ctx.pixels_per_point();
    let clipped_primitives = overlay.ctx.tessellate(full_output.shapes, pixels_per_point);

    let (width, height) = gpu.size();
    let screen_descriptor = ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point,
    };

    for (id, image_delta) in &full_output.textures_delta.set {
        overlay
            .renderer
            .update_texture(gpu.device(), gpu.queue(), *id, image_delta);
    }

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("egui_encoder"),
        });

    // #785 — what the editor's own interface costs on the GPU, kept
    // apart from the viewport passes above it. Someone asking "how
    // expensive is my scene" is asking about those; someone asking "why
    // is the editor slow" is usually asking about this one.
    let ui_query = scopes.as_ref().map(|s| s.begin("editor ui", &mut encoder));

    let extra_buffers = overlay.renderer.update_buffers(
        gpu.device(),
        gpu.queue(),
        &mut encoder,
        &clipped_primitives,
        &screen_descriptor,
    );

    // 🔴 Both swapchain calls carry a scope, because either one can be
    // the vblank wait and they are not interchangeable: `acquire` blocks
    // when no image is free, `present` blocks when the queue is full.
    // Unscoped, the wait landed outside the flamegraph entirely and the
    // profiler reported a 9.6 ms frame while the clock said 17.4.
    let output = {
        profiling::scope!("surface acquire");
        match gpu.surface().get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(tex)
            | wgpu::CurrentSurfaceTexture::Suboptimal(tex) => tex,
            status => {
                tracing::warn!(?status, "Failed to acquire surface texture");
                return false;
            }
        }
    };
    let view = output
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    {
        let render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("egui_render_pass"),
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

        let mut render_pass = render_pass.forget_lifetime();
        overlay
            .renderer
            .render(&mut render_pass, &clipped_primitives, &screen_descriptor);
    }

    if let (Some(scopes), Some(query)) = (scopes.as_ref(), ui_query) {
        scopes.end(&mut encoder, query);
    }
    // The frame's last encoder, so the timestamps every viewport wrote
    // are copied out here — they were submitted before this one and are
    // resolved on the queue by the time this copy runs.
    if let Some(scopes) = scopes.as_deref_mut() {
        scopes.resolve(&mut encoder);
    }

    let mut buffers = extra_buffers;
    buffers.push(encoder.finish());
    gpu.queue().submit(buffers);
    {
        profiling::scope!("surface present");
        output.present();
    }
    // After every submit of the frame, never between them.
    if let Some(scopes) = scopes {
        scopes.end_frame(gpu.queue());
    }

    for id in &full_output.textures_delta.free {
        overlay.renderer.free_texture(id);
    }

    true
}
