//! Offscreen wgpu texture bound to an egui texture id, sized to the
//! viewport panel and recreated on resize.

/// Minimum texture size in pixels (avoids zero-sized textures when the
/// panel is collapsed or not yet laid out).
const MIN_SIZE: u32 = 1;

pub(crate) struct ViewportTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    texture_id: egui::TextureId,
    format: wgpu::TextureFormat,
    current_size: (u32, u32),
    requested_size: (u32, u32),
}

impl ViewportTarget {
    pub fn new(
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
        format: wgpu::TextureFormat,
        size: (u32, u32),
    ) -> Self {
        let size = clamp_size(size);
        let (texture, view) = create_offscreen(device, format, size);
        let texture_id =
            egui_renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear);
        Self {
            texture,
            view,
            texture_id,
            format,
            current_size: size,
            requested_size: size,
        }
    }

    pub fn texture_id(&self) -> egui::TextureId {
        self.texture_id
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn aspect(&self) -> f32 {
        self.current_size.0 as f32 / self.current_size.1.max(1) as f32
    }

    /// Records a size request. The actual resize happens on the next call
    /// to [`resize_if_needed`], before the frame is rendered.
    pub fn request_size(&mut self, size: (u32, u32)) {
        self.requested_size = clamp_size(size);
    }

    /// If the requested size differs from the current backing texture, drops
    /// the old texture, recreates it, and re-registers it with egui.
    pub fn resize_if_needed(
        &mut self,
        device: &wgpu::Device,
        egui_renderer: &mut egui_wgpu::Renderer,
    ) {
        if self.requested_size == self.current_size {
            return;
        }
        egui_renderer.free_texture(&self.texture_id);
        let (tex, view) = create_offscreen(device, self.format, self.requested_size);
        self.texture = tex;
        self.view = view;
        self.texture_id =
            egui_renderer.register_native_texture(device, &self.view, wgpu::FilterMode::Linear);
        self.current_size = self.requested_size;
    }
}

fn clamp_size(size: (u32, u32)) -> (u32, u32) {
    (size.0.max(MIN_SIZE), size.1.max(MIN_SIZE))
}

fn create_offscreen(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    size: (u32, u32),
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport_offscreen_texture"),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
