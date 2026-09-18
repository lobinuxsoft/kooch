//! The post-process slot: one full-screen draw over the frame the camera produced (#1201).
//!
//! 🔴 It reads the finished colour and writes a target from the pool, which is then copied back. The
//! read and the write cannot be the same texture, and the pool is what hands out the second one
//! without allocating per frame.
//!
//! The shader is authored in the Shader Graph like a surface: same contract, same parameters, same
//! material. `kind: post_process` is what puts it here instead of on a mesh.

mod pipeline;

use kooch_core::gpu::{TargetDesc, TargetPool};

use crate::material::{MaterialPipeline, SurfaceSource};
use crate::meshlet::validate_post;

pub use pipeline::PostUniforms;

/// What a post-process draw needs from the frame around it.
pub struct PostFrame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub targets: &'a mut TargetPool,
    /// The finished colour: read here, and written back at the end.
    pub scene: &'a wgpu::Texture,
    pub scene_view: &'a wgpu::TextureView,
    pub size: (u32, u32),
    pub time: f32,
}

/// The slot, with the pipeline it built for the material it is showing.
pub struct PostPass {
    format: wgpu::TextureFormat,
    parts: pipeline::Parts,
    /// The shader the pipeline was built from, by material slot and revision, so an edit rebuilds
    /// and an unchanged frame does not.
    built: Option<(u32, u64)>,
    pipeline: Option<wgpu::RenderPipeline>,
    /// Why the last build failed, for the panel to show.
    refusal: Option<String>,
}

impl PostPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            parts: pipeline::Parts::new(device),
            built: None,
            pipeline: None,
            refusal: None,
        }
    }

    /// Why the shader did not compile, if it did not.
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// Runs the material's post-process over `frame.scene`. `false` when there is nothing to run —
    /// no material, a material without a `post_process` shader, or one that does not compile.
    ///
    /// The frame costs nothing when this returns `false`: no target is acquired and no pass opened.
    pub fn apply(
        &mut self,
        frame: PostFrame<'_>,
        materials: &MaterialPipeline,
        material: kooch_core::Guid,
    ) -> bool {
        let Some(slot) = materials.lookup(material) else {
            return false;
        };
        let Some((_, surface)) = materials.slot_surface(slot) else {
            return false;
        };
        if !self.ensure_pipeline(frame.device, slot, surface) {
            return false;
        }
        let Some(pipeline) = self.pipeline.as_ref() else {
            return false;
        };

        // Same size and format as what it reads, so the copy back is a plain texture copy.
        let desc = TargetDesc::attachment(frame.size, self.format).with_usage(
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let target = frame.targets.acquire("post_process", desc);
        let Some(view) = frame.targets.view(target).cloned() else {
            return false;
        };

        self.parts.write_uniforms(
            frame.queue,
            PostUniforms {
                resolution: [frame.size.0 as f32, frame.size.1 as f32],
                time: frame.time,
                material_id: slot,
            },
        );
        let groups = self
            .parts
            .bind_groups(frame.device, frame.scene_view, materials, slot);

        {
            let mut pass = frame
                .encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("post_process"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
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
            pass.set_pipeline(pipeline);
            for (index, group) in groups.iter().enumerate() {
                pass.set_bind_group(index as u32, group, &[]);
            }
            pass.draw(0..3, 0..1);
        }

        if let Some(texture) = frame.targets.texture(target) {
            frame.encoder.copy_texture_to_texture(
                texture.as_image_copy(),
                frame.scene.as_image_copy(),
                wgpu::Extent3d {
                    width: frame.size.0.max(1),
                    height: frame.size.1.max(1),
                    depth_or_array_layers: 1,
                },
            );
        }
        frame.targets.release(target);
        true
    }

    /// Builds the pipeline for `surface` when it is not the one already built. `false` when the
    /// shader is not a post-process or does not compile — the refusal is kept for the panel.
    fn ensure_pipeline(
        &mut self,
        device: &wgpu::Device,
        slot: u32,
        surface: &SurfaceSource,
    ) -> bool {
        if self.built == Some((slot, surface.revision)) {
            return self.pipeline.is_some();
        }
        self.built = Some((slot, surface.revision));
        self.pipeline = None;
        self.refusal = None;

        if !surface.source.contains("fn post_process(") {
            // Not a post-process shader: an ordinary material assigned here by mistake.
            return false;
        }
        // 🔴 Checked before a pipeline is built from it: a broken edit has to read as a message in
        // the panel, never as a wgpu validation panic.
        if let Err(why) = validate_post(&surface.params_wgsl, &surface.source) {
            self.refusal = Some(why);
            return false;
        }
        self.pipeline =
            Some(
                self.parts
                    .build(device, self.format, &surface.params_wgsl, &surface.source),
            );
        true
    }
}

#[cfg(test)]
mod tests;
