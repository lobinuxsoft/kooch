/// Approximate bytes occupied by the stage's three render targets at
/// the given resolution. Used by [`MeshletRenderStage::set_vram_tracker`]
/// to seed the counter with what `new()` already allocated.
pub(crate) fn render_target_byte_estimate(size: (u32, u32)) -> u64 {
    let pixels = size.0 as u64 * size.1 as u64;
    // vbuf: R32Uint = 4 bpp; depth: Depth32Float = 4 bpp;
    // color: Rgba8Unorm = 4 bpp. Total: 12 bytes/pixel.
    pixels * 12
}

/// Creates a `TextureAspect::DepthOnly` view of a depth attachment so
/// the same texture can be sampled by the Hi-Z builder while the
/// `_view` (with `aspect: All`) drives the render pass attachment.
pub(crate) fn depth_sample_view(texture: &wgpu::Texture) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("meshlet_render_stage_depth_sample"),
        format: Some(wgpu::TextureFormat::Depth32Float),
        dimension: Some(wgpu::TextureViewDimension::D2),
        usage: None,
        aspect: wgpu::TextureAspect::DepthOnly,
        base_mip_level: 0,
        mip_level_count: Some(1),
        base_array_layer: 0,
        array_layer_count: Some(1),
    })
}

pub(crate) fn create_2d_attachment(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}
