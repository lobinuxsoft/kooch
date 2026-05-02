//! GPU-resident texture wrapper.
//!
//! Holds a `wgpu::Texture` + view. Mipmap generation and sampler caching
//! are intentionally deferred — landed in their own follow-ups when PBR
//! materials need them.

use super::asset::Image;

/// GPU texture: device texture + default 2D view + descriptor metadata.
///
/// `wgpu::Texture` keeps the GPU memory alive; dropping `GpuTexture`
/// releases it. `view` is the standard 2D view used for shader sampling;
/// custom views (mip subset, array slice) build via the texture handle
/// directly.
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: wgpu::Extent3d,
    pub format: wgpu::TextureFormat,
}

impl GpuTexture {
    /// Uploads `image` to a freshly allocated GPU texture (single mip).
    /// Mipmap generation comes with the PBR material pass — mip `0` is
    /// what we feed today.
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image) -> Self {
        let size = wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        };
        let format = image.format.wgpu();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let bytes_per_row = image.width * image.format.bytes_per_pixel();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &image.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(image.height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size,
            format,
        }
    }
}
