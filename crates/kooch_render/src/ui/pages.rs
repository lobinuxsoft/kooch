//! The page atlas (#1893): large rasters and videos, on the GPU only. The list hands each raster
//! over once; growing copies the old texture on the GPU, since nothing else holds its texels.

use kooch_ecs::UiDrawList;

pub(super) struct UiPages {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    side: u32,
}

impl UiPages {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let texture = page_texture(device, 1);
        Self {
            view: texture.create_view(&Default::default()),
            texture,
            side: 1,
        }
    }

    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub(super) fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Resizes to the list's side and writes what it hands over. `(replaced, rewritten)`: a new
    /// texture needs a new bind group, and any write has every video slot drawn again.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        list: &mut UiDrawList,
    ) -> (bool, bool) {
        let replaced = list.page_side > 0 && list.page_side != self.side;
        if replaced {
            let texture = page_texture(device, list.page_side);
            let kept = self.side.min(list.page_side);
            encoder.copy_texture_to_texture(
                self.texture.as_image_copy(),
                texture.as_image_copy(),
                wgpu::Extent3d {
                    width: kept,
                    height: kept,
                    depth_or_array_layers: 1,
                },
            );
            self.view = texture.create_view(&Default::default());
            self.texture = texture;
            self.side = list.page_side;
        }
        let uploads = std::mem::take(&mut list.page_uploads);
        let written = !uploads.is_empty();
        for upload in uploads {
            let [x0, y0, x1, y1] = upload.rect;
            if x1 > self.side || y1 > self.side || x0 >= x1 || y0 >= y1 {
                continue;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: x0, y: y0, z: 0 },
                    aspect: wgpu::TextureAspect::All,
                },
                &upload.texels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some((x1 - x0) * 4),
                    rows_per_image: Some(y1 - y0),
                },
                wgpu::Extent3d {
                    width: x1 - x0,
                    height: y1 - y0,
                    depth_or_array_layers: 1,
                },
            );
        }
        (replaced, replaced || written)
    }
}

fn page_texture(device: &wgpu::Device, side: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ui_pages"),
        size: wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        // Grown by a GPU copy, and a video's frames are drawn into its slot.
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    })
}
