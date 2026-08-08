use super::*;

#[test]
fn from_rgba8_stores_data_and_dims() {
    let data = vec![10, 20, 30, 40, 50, 60, 70, 80];
    let img = Image::from_rgba8(data.clone(), 2, 1, ImageFormat::Rgba8Unorm);
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 1);
    assert_eq!(img.format, ImageFormat::Rgba8Unorm);
    assert_eq!(img.data, data);
    assert_eq!(img.byte_count(), 8);
}

#[test]
fn solid_color_is_one_pixel() {
    let img = Image::solid_color([255, 255, 255, 255], ImageFormat::Rgba8UnormSrgb);
    assert_eq!(img.width, 1);
    assert_eq!(img.height, 1);
    assert_eq!(img.data, vec![255, 255, 255, 255]);
}

#[test]
fn format_maps_to_wgpu() {
    assert_eq!(
        ImageFormat::Rgba8UnormSrgb.wgpu(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
    );
    assert_eq!(
        ImageFormat::Rgba8Unorm.wgpu(),
        wgpu::TextureFormat::Rgba8Unorm,
    );
}

#[test]
fn bytes_per_pixel_is_four_for_rgba8() {
    assert_eq!(ImageFormat::Rgba8UnormSrgb.bytes_per_pixel(), 4);
    assert_eq!(ImageFormat::Rgba8Unorm.bytes_per_pixel(), 4);
}
