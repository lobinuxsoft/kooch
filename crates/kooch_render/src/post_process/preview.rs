//! A post-process drawn over a built-in test image, for the Shader Graph panel (#1201): the effect
//! reads before it is assigned to anything.

use std::hash::{Hash, Hasher};

use super::pipeline::{Parts, PostUniforms, Uniforms};
use crate::material::shader::MAX_PARAM_TEXTURES;
use crate::material::{MaterialPool, MaterialTexturePool, TextureRef};
use crate::meshlet::validate_post;

/// Side of the test image, in pixels. Small on purpose: a pixelate or a dither shows at this size.
const TEST_SIDE: u32 = 256;

/// The pipeline, the test image it reads, and its uniforms.
pub struct PostPreview {
    format: wgpu::TextureFormat,
    parts: Parts,
    uniforms: Uniforms,
    scene: wgpu::TextureView,
    /// The shader it was built from, so an unchanged graph does not recompile every frame.
    pipeline: Option<(u64, wgpu::RenderPipeline)>,
}

/// The material a preview reads its parameters and textures from.
pub struct PreviewMaterials<'a> {
    pub pool: &'a MaterialPool,
    pub textures: &'a MaterialTexturePool,
    pub slots: &'a [TextureRef; MAX_PARAM_TEXTURES as usize],
}

impl PostPreview {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let parts = Parts::new(device);
        let uniforms = parts.uniforms(device);
        Self {
            format,
            scene: test_image(device, queue),
            parts,
            uniforms,
            pipeline: None,
        }
    }

    /// Draws the effect into `target`. `Err` carries why the shader does not compile, for the
    /// panel to show in place of the image.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        size: (u32, u32),
        params_wgsl: &str,
        source: &str,
        materials: PreviewMaterials<'_>,
        time: f32,
    ) -> Result<(), String> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        params_wgsl.hash(&mut hasher);
        source.hash(&mut hasher);
        let stamp = hasher.finish();
        if self
            .pipeline
            .as_ref()
            .is_none_or(|(built, _)| *built != stamp)
        {
            // 🔴 Checked before wgpu sees it: a graph mid-edit does not compile, and a
            // validation error there is a panic rather than a message.
            validate_post(params_wgsl, source)?;
            let pipeline = self.parts.build(device, self.format, params_wgsl, source);
            self.pipeline = Some((stamp, pipeline));
        }
        let Some((_, pipeline)) = self.pipeline.as_ref() else {
            return Ok(());
        };

        self.parts.write_uniforms(
            queue,
            &self.uniforms,
            PostUniforms {
                resolution: [size.0 as f32, size.1 as f32],
                time,
                material_id: 0,
                weight: 1.0,
            },
        );
        let groups = self.parts.bind_groups(
            device,
            &self.uniforms,
            &self.scene,
            materials.pool,
            materials.textures,
            materials.slots,
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post_process_preview"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        Ok(())
    }
}

/// A hue sweep across, a brightness ramp down, and a checker over one corner: gradients show
/// banding and dither, the checker shows pixelation and blur.
fn test_image(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::TextureView {
    let side = TEST_SIDE;
    let mut pixels = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            pixels.extend_from_slice(&test_pixel(x, y, side));
        }
    }
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("post_process_test_image"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &pixels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(side * 4),
            rows_per_image: Some(side),
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}

/// One pixel of the test image, as RGBA8.
fn test_pixel(x: u32, y: u32, side: u32) -> [u8; 4] {
    let (u, v) = (x as f32 / side as f32, y as f32 / side as f32);
    if u > 0.75 && v > 0.75 {
        let cell = ((x / 8) + (y / 8)) % 2;
        let level = if cell == 0 { 30 } else { 225 };
        return [level, level, level, 255];
    }
    let hue = hue(u);
    let brightness = 1.0 - v;
    let channel = |c: f32| (c * brightness * 255.0).round() as u8;
    [channel(hue[0]), channel(hue[1]), channel(hue[2]), 255]
}

/// A fully saturated colour at `t` around the wheel, 0..1.
fn hue(t: f32) -> [f32; 3] {
    let f = |n: f32| {
        let k = (n + t * 6.0) % 6.0;
        1.0 - (k.min(4.0 - k).clamp(0.0, 1.0))
    };
    [f(5.0), f(3.0), f(1.0)]
}

#[cfg(test)]
mod tests;
