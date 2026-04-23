//! Viewport ray-march pass: writes the current scene into the offscreen
//! texture, or clears it to black when there is nothing to render.

use glam::Vec4;

use ome_core::gpu::GpuContext;
use ome_core::resource::Resources;
use ome_ecs::SdfSphere;
use ome_ecs::hierarchy::GlobalTransform;
use ome_ecs::mesh_renderer::MeshRenderer;
use ome_ecs::query::Query;
use ome_render::{MeshPassRenderer, RayMarchRenderer};

use crate::viewport::target::ViewportTarget;

/// Sky gradient used when the scene has renderable content. Kept in sync
/// with the defaults of `SkyGradient` in `ome_render::raymarch_plugin`.
const SKY_TOP: Vec4 = Vec4::new(0.5, 0.7, 1.0, 1.0);
const SKY_BOTTOM: Vec4 = Vec4::new(0.1, 0.2, 0.4, 1.0);

/// Renders the active scene into the viewport offscreen texture.
///
/// Two passes share the encoder and the offscreen target:
/// 1. **Ray-march pass** — clears the target to sky / black and writes
///    visible SDF shapes (skipped when there is no active camera or no
///    SDF content; in that case a black clear pass runs instead).
/// 2. **Mesh pass** — paints visible `MeshRenderer + GlobalTransform`
///    entities on top of whatever the ray-march pass produced. Skipped
///    when no entity has a non-empty `mesh` path. No depth buffer yet
///    (issue #129 ships scope-strict; depth/SDF compositing follows).
pub(crate) fn render_viewport(
    gpu: &GpuContext,
    raymarch: &mut RayMarchRenderer,
    mesh_pass: &mut MeshPassRenderer,
    target: &ViewportTarget,
    resources: &Resources,
    project_loaded: bool,
) {
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("viewport_encoder"),
        });

    let has_sdf = project_loaded && has_visible_sdf(resources);
    let camera_ok = has_sdf
        && raymarch.update_camera(gpu.device(), gpu.queue(), resources, target.aspect());

    if camera_ok {
        raymarch.update_scene(gpu.device(), gpu.queue(), resources, SKY_TOP, SKY_BOTTOM);
        raymarch.render(&mut encoder, target.view());
    } else {
        clear_to_black(&mut encoder, target.view());
    }

    if project_loaded && has_visible_mesh(resources) {
        mesh_pass.render(
            gpu.device(),
            gpu.queue(),
            &mut encoder,
            target.view(),
            resources,
            target.aspect(),
        );
    }

    gpu.queue().submit(Some(encoder.finish()));
}

fn clear_to_black(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
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
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

fn has_visible_sdf(resources: &Resources) -> bool {
    let query = Query::<&SdfSphere>::new(resources);
    let mut found = false;
    query.for_each(|sphere| {
        if sphere.visible {
            found = true;
        }
    });
    found
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
