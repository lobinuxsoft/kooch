//! Everything that needs the swapchain image: the sky, the blit that composites the scene over it,
//! and the present (#392).

use kooch_core::event::{AppExit, Events};
use kooch_core::gpu::{GpuContext, TargetDesc, TargetId, TargetPool};
use kooch_core::resource::Resources;
use kooch_core::time::Time;
use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

use super::frame::FrameSetup;
use super::{GameDepth, SKY_FALLBACK};
use crate::meshlet::{MeshletBlit, MeshletRenderStage};
use crate::post_process::{StackTarget, active_stack, run_stack};
use crate::sky::SkyRenderPass;

pub(super) fn present_frame_system(resources: &mut Resources) {
    let Some(setup) = resources.get::<FrameSetup>().cloned() else {
        return;
    };
    let Some(mut sky_pass) = resources.remove::<SkyRenderPass>() else {
        return;
    };
    let Some(gpu) = resources.remove::<GpuContext>() else {
        resources.insert(sky_pass);
        return;
    };
    let Some(stage) = resources.remove::<MeshletRenderStage>() else {
        resources.insert(gpu);
        resources.insert(sky_pass);
        return;
    };
    let Some(blit) = resources.remove::<MeshletBlit>() else {
        resources.insert(gpu);
        resources.insert(sky_pass);
        resources.insert(stage);
        return;
    };
    let Some(depth) = resources.remove::<GameDepth>() else {
        resources.insert(gpu);
        resources.insert(sky_pass);
        resources.insert(stage);
        resources.insert(blit);
        return;
    };
    // Cloned out of the pool: the frame records with it while the pool goes on serving whoever else
    // asks. A view is a handle, so this is a refcount rather than a texture.
    let depth_view = resources
        .get::<TargetPool>()
        .and_then(|pool| depth.view(pool));
    let Some(depth_view) = depth_view else {
        resources.insert(gpu);
        resources.insert(sky_pass);
        resources.insert(stage);
        resources.insert(blit);
        resources.insert(depth);
        return;
    };

    let outcome = acquire_and_present(
        &gpu,
        &mut sky_pass,
        &stage,
        &blit,
        &depth_view,
        resources,
        &setup,
    );

    resources.insert(gpu);
    resources.insert(sky_pass);
    resources.insert(depth);
    resources.insert(stage);
    resources.insert(blit);
    if let Some(pool) = resources.get_mut::<TargetPool>() {
        pool.end_frame();
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

#[allow(clippy::too_many_arguments)]
fn acquire_and_present(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    meshlet_stage: &MeshletRenderStage,
    meshlet_blit: &MeshletBlit,
    depth_view: &wgpu::TextureView,
    resources: &mut Resources,
    setup: &FrameSetup,
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
                setup,
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

enum SurfaceOutcome {
    Presented,
    Skip,
    NeedsReconfigure,
    Error,
}

#[allow(clippy::too_many_arguments)]
/// Everything that needs the swapchain image, and nothing that does not.
fn render_passes(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    meshlet_stage: &MeshletRenderStage,
    meshlet_blit: &MeshletBlit,
    depth_view: &wgpu::TextureView,
    resources: &mut Resources,
    setup: &FrameSetup,
    frame: SurfaceTexture,
) {
    let aspect = setup.aspect;
    let surface_view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    // 🔴 A post-process samples what it reads, and a swapchain image cannot be sampled: with a
    // stack, the frame is drawn off-screen and copied in at the end. Without one, straight in.
    let stack = active_stack(resources);
    // The image's own size, not the configured one: a resize can land between the two.
    let size = (frame.texture.width(), frame.texture.height());
    let offscreen = offscreen_target(gpu, resources, &stack, size);
    let view = offscreen
        .as_ref()
        .map_or_else(|| surface_view.clone(), |(_, view)| view.clone());

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

    // The sky belongs to the base camera: an overlay brings none, which is what keeps the base
    // visible under it (#1221).
    let sky_drawn = if let (Some(active_sky), Some(camera)) = (
        SkyRenderPass::active_sky(resources),
        setup.stack.base.as_ref(),
    ) {
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

    // Composite the meshlet stage's color over the sky only when the stage has GPU-resident meshes.
    // Without this guard the blit would copy the stage's empty color buffer over the sky every
    // frame, blanking the surface to black until something is registered.
    if meshlet_stage.gpu_mesh_count() > 0 {
        let blit_query = scopes.map(|s| s.begin("blit", &mut encoder));
        meshlet_blit.blit(
            gpu.device(),
            &mut encoder,
            meshlet_stage.color_view(),
            meshlet_stage.depth_sample_view(),
            &view,
            depth_view,
        );
        // The overlays over it, lowest priority first: each keeps what it did not draw, so the
        // stack reads as one image (#1221).
        let overlays = resources
            .get::<crate::camera_stack::StackViews>()
            .map(|views| views.drawn(&setup.stack))
            .unwrap_or_default();
        for overlay in overlays {
            if let (Some(color), Some(depth)) = (
                meshlet_stage.view_color_view(overlay),
                meshlet_stage.view_depth_sample(overlay),
            ) {
                meshlet_blit.blit(gpu.device(), &mut encoder, color, depth, &view, depth_view);
            }
        }
        if let (Some(scopes), Some(query)) = (scopes, blit_query) {
            scopes.end(&mut encoder, query);
        }
    }

    if let Some((target, view)) = offscreen {
        finish_offscreen(
            gpu,
            &mut encoder,
            resources,
            &stack,
            target,
            &view,
            &frame.texture,
        );
    }

    // The frame's last encoder, so this is where the timestamps are copied out — including the
    // meshlet stage's, which were written into an encoder submitted before this one and are
    // therefore already resolved on the queue by the time this copy runs.
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

/// A pooled colour target the size and format of the swapchain, when there is a stack to run and
/// the surface can take the copy back.
fn offscreen_target(
    gpu: &GpuContext,
    resources: &mut Resources,
    stack: &[(kooch_core::Guid, f32)],
    size: (u32, u32),
) -> Option<(TargetId, wgpu::TextureView)> {
    if stack.is_empty() {
        return None;
    }
    if !gpu.surface_copyable() {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::warn!("the surface cannot be copied into; post-process is off in this window");
        });
        return None;
    }
    let pool = resources.get_mut::<TargetPool>()?;
    let desc = TargetDesc::attachment(size, gpu.format()).with_usage(
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
    );
    let target = pool.acquire("game_post_scene", desc);
    let view = pool.view(target).cloned()?;
    Some((target, view))
}

/// Runs the stack over the off-screen frame, then copies it onto the swapchain image.
fn finish_offscreen(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    resources: &mut Resources,
    stack: &[(kooch_core::Guid, f32)],
    target: TargetId,
    view: &wgpu::TextureView,
    surface: &wgpu::Texture,
) {
    let Some(texture) = resources
        .get::<TargetPool>()
        .and_then(|pool| pool.texture(target).cloned())
    else {
        return;
    };
    let size = (surface.width(), surface.height());
    let query = resources
        .get::<kooch_core::gpu::GpuScopes>()
        .map(|scopes| scopes.begin("post_process", encoder));
    run_stack(
        gpu.device(),
        gpu.queue(),
        encoder,
        StackTarget {
            texture: &texture,
            view,
            size,
            format: gpu.format(),
        },
        stack,
        resources,
    );
    encoder.copy_texture_to_texture(
        texture.as_image_copy(),
        surface.as_image_copy(),
        wgpu::Extent3d {
            width: size.0.max(1),
            height: size.1.max(1),
            depth_or_array_layers: 1,
        },
    );
    if let (Some(scopes), Some(query)) = (resources.get::<kooch_core::gpu::GpuScopes>(), query) {
        scopes.end(encoder, query);
    }
    if let Some(pool) = resources.get_mut::<TargetPool>() {
        pool.release(target);
    }
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
