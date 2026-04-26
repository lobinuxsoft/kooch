//! Viewport render orchestration.
//!
//! Three passes share one encoder and the offscreen target, in this order:
//! 1. **Sky pass** (optional) — runs when an active `SkyRenderer` entity
//!    exists. Clears color + depth and draws the procedural sky.
//! 2. **Ray-march pass** — draws SDF shapes. If the sky pass ran first,
//!    loads the targets (preserving the sky) and discards on ray miss;
//!    otherwise clears the targets and draws its internal gradient.
//! 3. **Mesh pass** — paints visible `MeshRenderer + GlobalTransform`
//!    entities on top, depth-testing against the SDF depth buffer.

use glam::Vec4;

use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use ome_core::time::Time;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::Query;
use ome_gizmos::{GizmoBatch, GizmoRenderer, MeshBatch, MeshGizmoRenderer};
use ome_render::{MeshPassRenderer, RayMarchRenderer, SkyRenderPass, has_any_visible_sdf};

use crate::viewport::target::ViewportTarget;

/// Fallback sky gradient used when no `SkyRenderer` entity exists.
/// Kept in sync with the defaults of `SkyGradient` in
/// `ome_render::raymarch_plugin` and the `SkyRenderer` component defaults.
const SKY_TOP: Vec4 = Vec4::new(0.5, 0.7, 1.0, 1.0);
const SKY_BOTTOM: Vec4 = Vec4::new(0.1, 0.2, 0.4, 1.0);

/// Renders the active scene into the viewport offscreen texture.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_viewport(
    gpu: &GpuContext,
    sky_pass: &mut SkyRenderPass,
    raymarch: &mut RayMarchRenderer,
    mesh_pass: &mut MeshPassRenderer,
    gizmo_renderer: &mut GizmoRenderer,
    gizmo_batch: &GizmoBatch,
    mesh_gizmo_renderer: &mut MeshGizmoRenderer,
    mesh_gizmo_batch: &MeshBatch,
    target: &ViewportTarget,
    resources: &Resources,
    project_loaded: bool,
) {
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

    // Pass 2: Ray-march.
    let has_sdf = project_loaded && has_any_visible_sdf(resources);
    let camera_ok = has_sdf
        && raymarch.update_camera(gpu.device(), gpu.queue(), resources, target.aspect());

    if camera_ok {
        raymarch.update_scene(
            gpu.device(),
            gpu.queue(),
            resources,
            SKY_TOP,
            SKY_BOTTOM,
            sky_drawn,
        );
        // When sky drew first, preserve its output; otherwise clear.
        raymarch.render(&mut encoder, target.view(), target.depth_view(), !sky_drawn);
    } else if !sky_drawn {
        clear_to_black(&mut encoder, target.view(), target.depth_view());
    }

    // Pass 3: Mesh.
    if project_loaded && has_visible_mesh(resources) {
        mesh_pass.render(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            target.view(),
            target.depth_view(),
            resources,
            target.aspect(),
        );
    }

    // Pass 4: Line gizmos (always-on-top, depth comparison `Always`,
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

    // Pass 5: Mesh gizmos (always-on-top, alpha-blended triangles for
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

fn has_visible_mesh(resources: &Resources) -> bool {
    let query = Query::<(&MeshRenderer, &GlobalTransform)>::new(resources);
    let mut found = false;
    query.for_each(|(mr, _)| {
        if mr.visible && !mr.mesh.is_empty() {
            found = true;
        }
    });
    found
}
