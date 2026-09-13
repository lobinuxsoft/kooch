//! GPU-resident texture wrapper.

use super::asset::Image;
use super::mipmap::{Mipmapper, level_count};

/// GPU texture: device texture + default 2D view + descriptor metadata.
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: wgpu::Extent3d,
    pub format: wgpu::TextureFormat,
}

impl GpuTexture {
    /// Uploads `image` to a freshly allocated GPU texture, **without a mip chain** whatever the
    /// image asks for.
    pub fn upload(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image) -> Self {
        Self::allocate(device, queue, image, 1)
    }

    /// Uploads `image` and builds its mip chain if `image.mipmaps`.
    pub fn upload_with(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &Image,
        mipmapper: &mut Mipmapper,
    ) -> Self {
        let levels = if image.mipmaps {
            level_count(image.width, image.height)
        } else {
            1
        };
        let texture = Self::allocate(device, queue, image, levels);
        mipmapper.generate(device, queue, &texture.texture);
        texture
    }

    fn allocate(device: &wgpu::Device, queue: &wgpu::Queue, image: &Image, levels: u32) -> Self {
        let size = wgpu::Extent3d {
            width: image.width,
            height: image.height,
            depth_or_array_layers: 1,
        };
        let format = image.format.wgpu();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("texture"),
            size,
            mip_level_count: levels.max(1),
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            // RENDER_ATTACHMENT only matters to the levels above zero, and it is requested
            // unconditionally.
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
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
