//! Render-path dispatch for [`RenderPlugin`]. Lives in its own module
//! so the plugin entry point ([`super`]) stays under the 400-LoC
//! ceiling and the legacy-vs-meshlet split is visible at a glance.

use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use ome_core::time::Time;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::perspective_camera::PerspectiveCamera;
use ome_ecs::query::Query;
use wgpu::{CurrentSurfaceTexture, SurfaceTexture};

use crate::mesh::MeshPassRenderer;
use crate::meshlet::{MeshletBlit, MeshletRenderStage};
use crate::sky::SkyRenderPass;

use super::SKY_FALLBACK;

pub(super) enum SurfaceOutcome {
    Presented,
    Skip,
    NeedsReconfigure,
    Error,
}

/// Selects which mesh pipeline runs after the sky pass / clear.
pub(super) enum RenderPath<'a> {
    Legacy,
    Meshlet {
        stage: &'a MeshletRenderStage,
        blit: &'a MeshletBlit,
    },
}

#[allow(clippy::too_many_arguments)]
pub(super) fn acquire_and_render(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    mesh_pass: &mut MeshPassRenderer,
    depth_view: &wgpu::TextureView,
    resources: &mut Resources,
    aspect: f32,
    path: RenderPath<'_>,
) -> SurfaceOutcome {
    match gpu.surface().get_current_texture() {
        CurrentSurfaceTexture::Success(tex) | CurrentSurfaceTexture::Suboptimal(tex) => {
            render_passes(
                gpu, sky_pass, mesh_pass, depth_view, resources, aspect, path, tex,
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
    mesh_pass: &mut MeshPassRenderer,
    depth_view: &wgpu::TextureView,
    resources: &mut Resources,
    aspect: f32,
    path: RenderPath<'_>,
    frame: SurfaceTexture,
) {
    let view = frame
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());

    // The meshlet stage submits its own command buffer (cull + raster
    // + deferred) before we record the surface-targeted encoder. Order
    // matters: blit reads the stage's color view, so the stage's
    // submit must complete first on the queue.
    if let RenderPath::Meshlet { stage, .. } = &path {
        run_meshlet_stage(gpu, stage, resources, aspect);
    }

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

    match path {
        RenderPath::Legacy => {
            if has_visible_legacy_mesh(resources) {
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
        }
        RenderPath::Meshlet { stage, blit } => {
            blit.blit(gpu.device(), &mut encoder, stage.color_view(), &view);
        }
    }

    gpu.queue().submit(Some(encoder.finish()));
    frame.present();
}

fn run_meshlet_stage(
    gpu: &GpuContext,
    stage: &MeshletRenderStage,
    resources: &Resources,
    aspect: f32,
) {
    let (view_proj, cam_pos) = active_camera_matrices(resources, aspect)
        .unwrap_or((glam::Mat4::IDENTITY, glam::Vec3::ZERO));
    let _ = stage.render_with_assets(gpu.device(), gpu.queue(), resources, view_proj, cam_pos);
}

fn active_camera_matrices(
    resources: &Resources,
    aspect: f32,
) -> Option<(glam::Mat4, glam::Vec3)> {
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut found: Option<(glam::Mat4, glam::Vec3)> = None;
    query.for_each(|(cam, gt)| {
        if found.is_some() || !cam.active {
            return;
        }
        let world = gt.matrix;
        let view = world.inverse();
        let fov_y_rad = cam.fov.to_radians().max(1.0_f32.to_radians());
        let proj = glam::Mat4::perspective_rh(
            fov_y_rad,
            aspect.max(0.01),
            cam.near.max(0.001),
            cam.far.max(cam.near + 0.001),
        );
        let cam_pos = world.w_axis.truncate();
        found = Some((proj * view, cam_pos));
    });
    found
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

fn has_visible_legacy_mesh(resources: &Resources) -> bool {
    let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
    let mut found = false;
    query.for_each(|(mr, _)| {
        if mr.visible && !mr.mesh.is_empty() {
            found = true;
        }
    });
    found
}
