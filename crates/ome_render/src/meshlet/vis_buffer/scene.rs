//! Scene-path render impl. Lives in its own file so the rasterizer
//! struct stays under the 400-LoC ceiling. The shader entry points
//! `vs_vbuf_scene` / `fs_vbuf_scene` are defined inside
//! `shaders/meshlet_vbuf.wgsl`; this module only records the per-frame
//! draw and builds the per-frame bind groups.

use bytemuck::bytes_of;

use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::scene::MeshletScene;

use super::{CameraUbo, MeshletVisRasterizer};

impl MeshletVisRasterizer {
    /// Scene-path render: rasterizes every visible (instance, meshlet)
    /// pair the cull dispatch surfaced into the visibility buffer.
    /// Per-instance transforms live inside `scene`'s instance buffer;
    /// the vertex shader fetches them through the pre-decoded packed
    /// values in `cull.visible_meshlets_buffer()`.
    ///
    /// `clear_id` is the visibility-buffer clear value (pass `0` for
    /// "background"). The depth attachment is mandatory for the scene
    /// path — multiple instances at different depths absolutely need
    /// it.
    ///
    /// `clear` selects the load op for both attachments. Use `true`
    /// for the first raster of a frame and `false` for any append
    /// pass (e.g. Hi-Z 2-pass cull's pass B raster, #445) so prior
    /// fragments survive.
    #[allow(clippy::too_many_arguments)]
    pub fn render_scene(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        view_proj: glam::Mat4,
        clear_id: u32,
        clear: bool,
    ) {
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );

        let visible_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_vbuf_scene_visible_bg"),
            layout: &self.visible_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: cull.visible_meshlets_buffer().as_entire_binding(),
            }],
        });
        let instances_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_vbuf_scene_instances_bg"),
            layout: &self.instances_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene.instance_buffer().as_entire_binding(),
            }],
        });

        let color_load = if clear {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: clear_id as f64,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            })
        } else {
            wgpu::LoadOp::Load
        };
        let depth_load = if clear {
            wgpu::LoadOp::Clear(0.0)
        } else {
            wgpu::LoadOp::Load
        };

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("meshlet_vbuf_scene_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: vbuf_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: color_load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline_scene);
        pass.set_bind_group(0, &self.camera_bg, &[]);
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, &visible_bg, &[]);
        pass.set_bind_group(3, &instances_bg, &[]);
        pass.draw_indirect(cull.indirect_args_buffer(), 0);
    }
}
