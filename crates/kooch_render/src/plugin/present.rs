//! Everything that needs the swapchain image: the sky, the blit that composites the scene over it,
//! and the present (#392).

use kooch_core::event::{AppExit, Events};
use kooch_core::gpu::{GpuContext, TargetPool};
use kooch_core::resource::Resources;
use kooch_core::time::Time;
use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

use super::frame::FrameSetup;
use super::{GameDepth, SKY_FALLBACK};
use crate::meshlet::{MeshletBlit, MeshletRenderStage};
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
                setup.aspect,
                tex,
                setup.camera.clone(),
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
    aspect: f32,
    frame: SurfaceTexture,
    camera: Option<crate::ViewCamera>,
) {
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

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
        if let (Some(scopes), Some(query)) = (scopes, blit_query) {
            scopes.end(&mut encoder, query);
        }
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
