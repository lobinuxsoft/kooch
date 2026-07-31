//! Scene-path deferred shading — wraps the `cs_shade_scene` compute
//! entry. Lives next to `mod.rs` so the deferred submodule keeps each
//! file under the 400-LoC ceiling and makes the path-vs-path split
//! visible at a glance.

use bytemuck::bytes_of;

use crate::meshlet::dispatcher::MeshletCull;
use crate::meshlet::scene::MeshletScene;

use super::{CameraUbo, MeshletDeferredShader, ScreenUbo};

impl MeshletDeferredShader {
    /// Records `cs_shade_scene` for the entire output. Walks every
    /// pixel; background pixels (cleared id 0) keep the clear color.
    /// Foreground pixels resolve their `(instance_id, meshlet_idx)`
    /// via `cull.visible_meshlets_buffer()` and read the per-instance
    /// transform + material id from `scene.instance_buffer()`.
    ///
    /// Unlike [`Self::shade`], this method does NOT take a per-call
    /// `model` matrix or `material_id` — those are per-instance state
    /// the shader pulls from the scene buffer.
    ///
    /// `debug_mode` is the raw [`MeshletDebugMode`](crate::meshlet::MeshletDebugMode)
    /// discriminant; 0 selects the production path. The shader
    /// branches on this uniform without an extra dispatch.
    #[allow(clippy::too_many_arguments)]
    pub fn shade_scene(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        vbuf_view: &wgpu::TextureView,
        color_view: &wgpu::TextureView,
        meshlet_bg: &wgpu::BindGroup,
        material_bg: &wgpu::BindGroup,
        cull: &MeshletCull,
        scene: &MeshletScene,
        view_proj: glam::Mat4,
        screen_size: (u32, u32),
        debug_mode: u32,
    ) {
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytes_of(&CameraUbo {
                view_proj: view_proj.to_cols_array_2d(),
            }),
        );
        // model UBO stays at whatever shade()/the previous frame left
        // there — `cs_shade_scene` ignores it. screen UBO needs the
        // size at minimum; material_id is unused on this path.
        queue.write_buffer(
            &self.screen_buffer,
            0,
            bytes_of(&ScreenUbo {
                size: [screen_size.0, screen_size.1],
                material_id: 0,
                debug_mode,
            }),
        );

        let shading_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_deferred_scene_shading_bg"),
            layout: &self.shading_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.model_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.screen_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(vbuf_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(color_view),
                },
            ],
        });
        let scene_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("meshlet_deferred_scene_bg"),
            layout: &self.scene_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: cull.visible_meshlets_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: scene.instance_buffer().as_entire_binding(),
                },
            ],
        });

        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("meshlet_deferred_scene_pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline_scene);
        pass.set_bind_group(0, &shading_bg, &[]);
        pass.set_bind_group(1, meshlet_bg, &[]);
        pass.set_bind_group(2, material_bg, &[]);
        pass.set_bind_group(3, &scene_bg, &[]);
        pass.dispatch_workgroups(screen_size.0.div_ceil(8), screen_size.1.div_ceil(8), 1);
    }
}
