//! Viewport render orchestration.
//!
//! Post-pivot 2026-05-02 (Plan C): mesh-only render. SDF raymarch pass
//! removed; voxel + DC pipeline (Phase 2.5) will feed mesh chunks into
//! the meshlet stage when ready.
//!
//! Passes share one encoder and the offscreen target, in this order:
//! 1. **Sky pass** (optional) — runs when an active `SkyRenderer` entity
//!    exists. Clears color + depth and draws the procedural sky.
//! 2. **Meshlet blit** — composites the meshlet stage's color output
//!    (cull + visibility raster + deferred shade) onto the viewport.
//! 3. **Gizmo passes** — line + mesh gizmos, always on top.

use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use ome_core::time::Time;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::query::Query;
use ome_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use ome_render::SkyRenderPass;
use ome_render::meshlet::{MeshletBlit, MeshletRenderStage, MeshletRenderStats};

use crate::viewport::target::ViewportTarget;

/// Inputs for the meshlet path. The stage drives cull + raster +
/// deferred; the blit composes it onto the `ViewportTarget`'s color view.
pub(crate) struct MeshletPathInputs<'a> {
    pub stage: &'a mut MeshletRenderStage,
    pub blit: &'a MeshletBlit,
}

/// Renders the active scene into the viewport offscreen texture.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_viewport(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    gizmo_renderer: &mut GizmoRenderer,
    gizmo_batch: &GizmoBatch,
    mesh_gizmo_renderer: &mut MeshGizmoRenderer,
    mesh_gizmo_batch: &MeshBatch,
    target: &ViewportTarget,
    resources: &mut Resources,
    project_loaded: bool,
    meshlet: MeshletPathInputs<'_>,
) {
    // The meshlet stage submits its own command buffer (cull + raster
    // + deferred). Order matters: the blit pass we record below reads
    // the stage's color view, so the stage's submit must complete
    // first on the queue.
    if project_loaded {
        meshlet.stage.resize(gpu.device(), target.size());
        meshlet.stage.sync_assets_to_gpu(gpu.device(), resources);
        let (view_proj, cam_pos) = active_camera_matrices(resources, target.aspect())
            .unwrap_or((glam::Mat4::IDENTITY, glam::Vec3::ZERO));
        let stats = meshlet.stage.render_with_assets(
            gpu.device(),
            gpu.queue(),
            resources,
            view_proj,
            cam_pos,
        );
        // Republish per-frame so the editor's debug-stats overlay (#451)
        // can read it next tick. Stats from a frame the meshlet stage
        // skipped (no project loaded) reset to default.
        resources.insert(stats);
    } else {
        resources.insert(MeshletRenderStats::default());
    }

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport_encoder"),
        });

    // Pass 1: Sky (when available).
    let sky_drawn = if project_loaded {
        if let Some(active_sky) = SkyRenderPass::active_sky(resources) {
            let time_secs = resources
                .get::<Time>()
                .map(|t| t.elapsed_secs())
                .unwrap_or(0.0);
            sky_pass.render(
                gpu.queue(),
                &mut encoder,
                target.view(),
                target.depth_view(),
                resources,
                target.aspect(),
                active_sky,
                time_secs,
            )
        } else {
            false
        }
    } else {
        false
    };

    if !sky_drawn {
        clear_to_black(&mut encoder, target.view(), target.depth_view());
    }

    // Pass 2: Meshlet blit composite — only when the stage actually
    // has GPU-resident meshes to draw. Otherwise the blit would copy
    // the stage's empty color buffer (all zeros) on top of the sky we
    // just rendered, blanking the viewport.
    if project_loaded && meshlet.stage.gpu_mesh_count() > 0 {
        meshlet
            .blit
            .blit(gpu.device(), &mut encoder, meshlet.stage.color_view(), target.view());
    }

    // Pass 3: Line gizmos (always-on-top, depth comparison `Always`,
    // screen-space thick lines).
    if project_loaded {
        gizmo_renderer.render(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            target.view(),
            target.depth_view(),
            resources,
            gizmo_batch,
            target.size(),
        );
    }

    // Pass 4: Mesh gizmos (always-on-top, alpha-blended triangles for
    // filled plane handles, future rotate tori, custom 3D shapes).
    if project_loaded {
        mesh_gizmo_renderer.render(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            target.view(),
            target.depth_view(),
            resources,
            mesh_gizmo_batch,
            target.size(),
        );
    }

    gpu.queue().submit(Some(encoder.finish()));
}

fn clear_to_black(
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    depth: &wgpu::TextureView,
) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("viewport_clear_pass"),
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

fn active_camera_matrices(
    resources: &Resources,
    aspect: f32,
) -> Option<(glam::Mat4, glam::Vec3)> {
    use ome_ecs::perspective_camera::PerspectiveCamera;

    // Pick the highest-priority active camera. The editor camera ships
    // with `priority = EDITOR_CAMERA_PRIORITY (1000)` so it outranks
    // user-spawned `PerspectiveCamera` defaults whenever both are
    // active simultaneously.
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, glam::Mat4, glam::Vec3)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        if let Some((p, _, _)) = best
            && cam.priority <= p
        {
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
        best = Some((cam.priority, proj * view, cam_pos));
    });
    best.map(|(_, vp, p)| (vp, p))
}
