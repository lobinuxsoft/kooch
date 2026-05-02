//! Helpers for the fragment-shader visual AC tests
//! (`raymarch_renders_sphere`, `gdf_fragment_sample`). Owns the
//! offscreen target build-up + camera/scene-meta uniform writes +
//! the readback path so each test stays focused on its assertion.
//!
//! Split out of `gdf_fragment_sample.rs` to keep that file under the
//! 400-LoC monolithic threshold. Mirrors the pattern `tests/common/gdf.rs`
//! uses for the populate-pass integration tests.

#![allow(dead_code)] // Each test binary touches a different subset.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use ome_bvh::sdf_primitive::{SdfPrimitive, TYPE_SPHERE};
use ome_bvh::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};
use ome_render::raymarch::RayMarchRenderer;
use ome_render::tile_cull::TileBounds;
use ome_world::{ChunkContent, ChunkId};

pub const TARGET_SIZE: u32 = 64;
pub const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Mirror of `raymarch::instance::CameraUniforms`. The pinned size at
/// the bottom of the consumer test trips this layout if the renderer
/// struct changes underneath us.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub struct CameraUniforms {
    pub view: [[f32; 4]; 4],
    pub projection: [[f32; 4]; 4],
    pub inverse_view: [[f32; 4]; 4],
    pub inverse_projection: [[f32; 4]; 4],
    pub position: [f32; 3],
    pub _pad0: f32,
}

/// Mirror of `raymarch::instance::SceneMeta`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub struct SceneMeta {
    pub primitive_count: u32,
    pub bvh_n: u32,
    pub skip_internal_sky: u32,
    pub has_intersects: u32,
    pub has_subs: u32,
    pub k_int_scene: f32,
    pub k_sub_scene: f32,
    pub _pad0: u32,
    pub sky_top: [f32; 4],
    pub sky_bottom: [f32; 4],
}

/// Configure a single-sphere chunk and write it through the
/// production renderer entry points. Caller controls camera + sphere
/// position + radius so each test exercises a distinct geometry.
pub fn setup_sphere_scene(
    renderer: &mut RayMarchRenderer,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    sphere_centre: Vec3,
    radius: f32,
    chunk_id: ChunkId,
) {
    let content = ChunkContent {
        primitives: vec![SdfPrimitive {
            position: sphere_centre.to_array(),
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: 0.0,
            params: [radius, 0.0, 0.0, 0.0],
        }],
        leaf_aabbs: vec![LeafAabb {
            aabb_min: [
                sphere_centre.x - radius,
                sphere_centre.y - radius,
                sphere_centre.z - radius,
            ],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [
                sphere_centre.x + radius,
                sphere_centre.y + radius,
                sphere_centre.z + radius,
            ],
            entity_id: chunk_id.coords.x as u32,
        }],
        max_smoothness_radius: 0.0,
    };
    renderer
        .bvh_state_mut()
        .insert_streaming_chunk(queue, chunk_id, &content)
        .expect("insert chunk");
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("common::raymarch_render::setup_sphere_scene"),
    });
    renderer
        .bvh_state_mut()
        .tick_uniforms(queue, &mut encoder, 0.0, 0.0);
    queue.submit(std::iter::once(encoder.finish()));
}

/// Write the camera uniforms + a default `SceneMeta` (sky enabled, no
/// intersects/subs) to the renderer.
pub fn write_camera_and_meta(
    renderer: &RayMarchRenderer,
    queue: &wgpu::Queue,
    camera_pos: Vec3,
    target: Vec3,
) {
    let view = Mat4::look_at_rh(camera_pos, target, Vec3::Y);
    let projection = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let cam = CameraUniforms {
        view: view.to_cols_array_2d(),
        projection: projection.to_cols_array_2d(),
        inverse_view: view.inverse().to_cols_array_2d(),
        inverse_projection: projection.inverse().to_cols_array_2d(),
        position: camera_pos.to_array(),
        _pad0: 0.0,
    };
    renderer.write_camera_uniforms(queue, bytemuck::bytes_of(&cam));

    let sky_top = Vec4::new(0.45, 0.65, 1.0, 1.0);
    let sky_bottom = Vec4::new(0.85, 0.92, 1.0, 1.0);
    let meta = SceneMeta {
        primitive_count: 1,
        bvh_n: 0,
        skip_internal_sky: 0,
        has_intersects: 0,
        has_subs: 0,
        k_int_scene: 0.0,
        k_sub_scene: 0.0,
        _pad0: 0,
        sky_top: sky_top.to_array(),
        sky_bottom: sky_bottom.to_array(),
    };
    renderer.write_scene_meta(queue, bytemuck::bytes_of(&meta));
}

/// Offscreen color + depth attachments + staging buffer the read-back
/// path needs. Reusable across visual AC tests.
pub struct OffscreenTargets {
    pub color_tex: wgpu::Texture,
    pub color_view: wgpu::TextureView,
    pub depth_view: wgpu::TextureView,
    pub staging: wgpu::Buffer,
    pub bytes_per_row: u32,
}

pub fn make_offscreen(device: &wgpu::Device) -> OffscreenTargets {
    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("common::raymarch_render::color"),
        size: wgpu::Extent3d {
            width: TARGET_SIZE,
            height: TARGET_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COLOR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("common::raymarch_render::depth"),
        size: wgpu::Extent3d {
            width: TARGET_SIZE,
            height: TARGET_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = depth_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let bytes_per_row = TARGET_SIZE * 4;
    assert_eq!(bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("common::raymarch_render::staging"),
        size: (bytes_per_row * TARGET_SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    OffscreenTargets {
        color_tex,
        color_view,
        depth_view,
        staging,
        bytes_per_row,
    }
}

/// Render the scene off-screen and read the framebuffer back to a
/// flat `[height][width][rgba]` byte vector.
pub fn render_and_readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &RayMarchRenderer,
    targets: &OffscreenTargets,
) -> Vec<u8> {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("common::raymarch_render::render_and_readback"),
    });
    renderer.render(&mut encoder, &targets.color_view, &targets.depth_view, true);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &targets.color_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &targets.staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(targets.bytes_per_row),
                rows_per_image: Some(TARGET_SIZE),
            },
        },
        wgpu::Extent3d {
            width: TARGET_SIZE,
            height: TARGET_SIZE,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = targets.staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        sender.send(res).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
    receiver
        .recv()
        .expect("map_async sender dropped")
        .expect("map_async failed");
    let pixels = {
        let view = slice.get_mapped_range();
        view.to_vec()
        // `view` (and the implicit lock on `slice`) drops here so
        // the subsequent `unmap()` call doesn't panic.
    };
    targets.staging.unmap();
    pixels
}

/// "Surface-like" pixel test (warm: R > B + 8). Mirrors the rule used
/// by `raymarch_renders_sphere` so the two AC suites share one
/// definition.
pub fn pixel_is_surface(pixel: [u8; 4]) -> bool {
    pixel[0] as i32 > pixel[2] as i32 + 8
}

/// "Sky-like" pixel test (cool: B > R). Used by ad-hoc debug runs,
/// kept here so the symbol stays in scope without `#[cfg(test)]`
/// gymnastics.
pub fn pixel_is_sky(pixel: [u8; 4]) -> bool {
    pixel[2] as i32 > pixel[0] as i32
}

pub fn pixel_at(pixels: &[u8], bytes_per_row: u32, x: u32, y: u32) -> [u8; 4] {
    let idx = ((y * bytes_per_row) + x * 4) as usize;
    [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
}

/// Run a tile-cull dispatch off the renderer's stored viewport size +
/// read the resulting SSBO back into a `Vec<TileBounds>` indexed
/// `[tile_y * tile_count_x + tile_x]`. Caller must have driven a
/// camera + GDF populate before calling — the compute samples cascade
/// 5 of `GdfState`, so unpopulated cascades produce empty tiles.
pub fn dispatch_and_readback_tile_bounds(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    renderer: &mut RayMarchRenderer,
) -> Vec<TileBounds> {
    renderer.dispatch_tile_cull(device, queue);
    let state = renderer.tile_cull_state();
    let total = state.last_uniforms().tile_count_total();
    assert!(total > 0, "tile_count_total must be > 0 — call set_viewport_size first");
    let entry_bytes = std::mem::size_of::<TileBounds>() as u64;
    let total_bytes = u64::from(total) * entry_bytes;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("common::raymarch_render::tile_bounds_staging"),
        size: total_bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("common::raymarch_render::tile_bounds_readback"),
    });
    encoder.copy_buffer_to_buffer(
        state.tile_bounds_buffer(),
        0,
        &staging,
        0,
        total_bytes,
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |res| {
        sender.send(res).ok();
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .expect("device poll");
    receiver
        .recv()
        .expect("map_async sender dropped")
        .expect("map_async failed");
    let bounds: Vec<TileBounds> = {
        let view = slice.get_mapped_range();
        bytemuck::cast_slice::<u8, TileBounds>(&view).to_vec()
    };
    staging.unmap();
    bounds
}
