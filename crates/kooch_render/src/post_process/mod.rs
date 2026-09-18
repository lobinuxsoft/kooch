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

/// One effect of the stack: its pipeline, the shader revision it was built from, and its uniforms.
struct Effect {
    revision: u64,
    pipeline: Option<wgpu::RenderPipeline>,
    uniforms: pipeline::Uniforms,
}

/// The slot, with a pipeline per effect it has shown.
pub struct PostPass {
    format: wgpu::TextureFormat,
    parts: pipeline::Parts,
    /// By material slot. CPU-side coordination, looked up once per effect per frame; a stack is a
    /// handful of entries, never a hot loop.
    effects: std::collections::HashMap<u32, Effect>,
    /// Why the last build failed, for the panel to show.
    refusal: Option<String>,
}

impl PostPass {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self {
            format,
            parts: pipeline::Parts::new(device),
            effects: std::collections::HashMap::new(),
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
        let Some(effect) = self.effects.get(&slot) else {
            return false;
        };
        let Some(pipeline) = effect.pipeline.as_ref() else {
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
            &effect.uniforms,
            PostUniforms {
                resolution: [frame.size.0 as f32, frame.size.1 as f32],
                time: frame.time,
                material_id: slot,
            },
        );
        let groups = self.parts.bind_groups(
            frame.device,
            &effect.uniforms,
            frame.scene_view,
            materials,
            slot,
        );

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

    /// Builds the pipeline for `surface` when this effect has none for its revision. `false` when
    /// the shader is not a post-process or does not compile — the refusal is kept for the panel.
    fn ensure_pipeline(
        &mut self,
        device: &wgpu::Device,
        slot: u32,
        surface: &SurfaceSource,
    ) -> bool {
        if let Some(effect) = self.effects.get(&slot)
            && effect.revision == surface.revision
        {
            return effect.pipeline.is_some();
        }
        let pipeline = self.build(device, surface);
        let uniforms = match self.effects.remove(&slot) {
            Some(effect) => effect.uniforms,
            None => self.parts.uniforms(device),
        };
        let built = pipeline.is_some();
        self.effects.insert(
            slot,
            Effect {
                revision: surface.revision,
                pipeline,
                uniforms,
            },
        );
        built
    }

    fn build(
        &mut self,
        device: &wgpu::Device,
        surface: &SurfaceSource,
    ) -> Option<wgpu::RenderPipeline> {
        if !surface.source.contains("fn post_process(") {
            // Not a post-process shader: an ordinary material put in the stack by mistake.
            return None;
        }
        // 🔴 Checked before a pipeline is built from it: a broken edit has to read as a message in
        // the panel, never as a wgpu validation panic.
        if let Err(why) = validate_post(&surface.params_wgsl, &surface.source) {
            self.refusal = Some(why);
            return None;
        }
        self.refusal = None;
        Some(
            self.parts
                .build(device, self.format, &surface.params_wgsl, &surface.source),
        )
    }
}

#[cfg(test)]
mod tests;
