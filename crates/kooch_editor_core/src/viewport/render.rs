//! Viewport render orchestration.

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_core::time::Time;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::query::Query;
use kooch_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use kooch_render::SkyRenderPass;
use kooch_render::meshlet::{MeshletBlit, MeshletRenderStage, MeshletRenderStats};

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
    // The meshlet stage submits its own command buffer (cull + raster + deferred). Order matters:
    // the blit pass we record below reads the stage's color view, so the stage's submit must
    // complete first on the queue.
    let frame_stats = if project_loaded {
        // Only the resize: this view's size is its own. Bringing the shared pools up to date is the
        // FRAME's job and happens once, ahead of both views — see `render_editor_frame` (#1171).
        meshlet.stage.resize(gpu.device(), target.size());
        let camera = view_camera(resources).unwrap_or_default();
        let stats = meshlet.stage.render_with_assets_primary(
            gpu.device(),
            gpu.queue(),
            resources,
            &camera,
            target.aspect(),
        );
        // Republish per-frame so the editor's debug-stats overlay (#451)
        // can read it next tick. Stats from a frame the meshlet stage
        // skipped (no project loaded) reset to default.
        resources.insert(stats);
        stats
    } else {
        let s = MeshletRenderStats::default();
        resources.insert(s);
        s
    };

    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport_encoder"),
        });

    // in the editor names the same pass. It is the one that measured 39.6 ms of a 71.6 ms frame on
    // the handheld, and the reason to have it here is that this is where the project is being
    // authored.
    let scopes = resources.get::<kooch_core::gpu::GpuScopes>();
    let sky_query = scopes.map(|s| s.begin("sky", &mut encoder));

    // Pass 1: Sky (when available).
    let sky_drawn = if project_loaded {
        if let (Some(active_sky), Some(camera)) =
            (SkyRenderPass::active_sky(resources), view_camera(resources))
        {
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
                &camera,
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
    if let (Some(scopes), Some(query)) = (scopes, sky_query) {
        scopes.end(&mut encoder, query);
    }

    // Pass 2: Meshlet blit composite — only when this frame actually produced meshlet output.
    // Gating on `gpu_mesh_count > 0` was wrong: that counts assets registered in the pool (which
    // persist after entities are despawned), not the live ECS instances that drove a real submit.
    if project_loaded && frame_stats.instances_uploaded > 0 {
        meshlet.blit.blit(
            gpu.device(),
            &mut encoder,
            meshlet.stage.color_view(),
            meshlet.stage.depth_sample_view(),
            target.view(),
            target.depth_view(),
        );
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
            &crate::gizmos::grid_planes(resources),
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

pub(super) fn clear_to_black(
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

/// The View panel's camera: the editor's own, always.
fn view_camera(resources: &Resources) -> Option<kooch_render::ViewCamera> {
    use crate::editor_camera::markers::EditorCamera;
    use kooch_ecs::perspective_camera::PerspectiveCamera;
    use kooch_ecs::query::filter::With;

    let editor =
        Query::<(&PerspectiveCamera, &GlobalTransform), With<EditorCamera>>::new(resources);
    let mut editor_cam: Option<kooch_render::ViewCamera> = None;
    editor.for_each(|(cam, gt)| {
        if editor_cam.is_none() {
            editor_cam = Some(kooch_render::ViewCamera::from_components(cam, gt));
        }
    });
    if editor_cam.is_some() {
        return editor_cam;
    }

    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, kooch_render::ViewCamera)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        if let Some((p, _)) = best
            && cam.priority <= p
        {
            return;
        }
        best = Some((
            cam.priority,
            kooch_render::ViewCamera::from_components(cam, gt),
        ));
    });
    best.map(|(_, camera)| camera)
}
