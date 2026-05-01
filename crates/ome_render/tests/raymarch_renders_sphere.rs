//! AC visual headless — pins what `pool_eval_smoke.rs` and `ac2_*` do
//! NOT exercise: the actual fragment-shader ray-march loop renders
//! visible geometry from a camera that lives outside every primitive
//! AABB.
//!
//! Lesson from session N5 of epic #370 (`feedback_gpu_driven_spirit.md`,
//! `feedback_planet_scale_gpu_driven.md`): point-eval AC tests pass
//! whenever CPU and GPU share the same bug, and visual verification
//! by eye misses regressions across PR boundaries. This test runs the
//! production fragment pipeline against a one-sphere scene and asserts
//! that the framebuffer pixel covering the sphere comes out non-sky.
//! Any reintroduction of the `aabb_contains(p)` point-query bug, the
//! TLAS slot-indexing regression, or a future shader rewrite that
//! breaks the camera-outside-scene case fails this test on CI before
//! it can ship.
//!
//! Headless: offscreen `Rgba8Unorm` 64×64 target + `Depth32Float`
//! depth, no window. Skipped silently when no Vulkan / Metal / DX12
//! adapter is available (mirrors the `try_acquire` pattern of every
//! other GPU integration test in this crate).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};
use ome_bvh::sdf_primitive::{SdfPrimitive, TYPE_SPHERE};
use ome_bvh::{IS_RAYMARCH, LeafAabb, ROLE_RAYMARCH_ADD};
use ome_world::{ChunkContent, ChunkId};

mod common;
use common::try_acquire_device;

const TARGET_SIZE: u32 = 64;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Mirror of `raymarch::instance::CameraUniforms`. The pinned size at
/// the bottom of the test trips this layout if the renderer struct
/// changes underneath us.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
struct CameraUniforms {
    view: [[f32; 4]; 4],
    projection: [[f32; 4]; 4],
    inverse_view: [[f32; 4]; 4],
    inverse_projection: [[f32; 4]; 4],
    position: [f32; 3],
    _pad0: f32,
}

/// Mirror of `raymarch::instance::SceneMeta`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
struct SceneMeta {
    primitive_count: u32,
    bvh_n: u32,
    skip_internal_sky: u32,
    has_intersects: u32,
    has_subs: u32,
    k_int_scene: f32,
    k_sub_scene: f32,
    _pad0: u32,
    sky_top: [f32; 4],
    sky_bottom: [f32; 4],
}

#[test]
fn raymarch_pipeline_renders_sphere_from_camera_outside_aabb() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("raymarch_renders_sphere — no GPU adapter, skipping");
        return;
    };

    // --- Renderer + scene setup --------------------------------------
    let mut renderer =
        ome_render::raymarch::RayMarchRenderer::new(&device, &queue, COLOR_FORMAT, None);

    // Single sphere at origin, radius 1.0, role-ADD. The leaf AABB is
    // tight to the sphere — this is the exact configuration that the
    // `aabb_contains(p)` point-query bug used to render as empty sky
    // because the camera at (0, 0, 5) lives outside the leaf AABB.
    let content = ChunkContent {
        primitives: vec![SdfPrimitive {
            position: [0.0, 0.0, 0.0],
            type_tag: TYPE_SPHERE,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0, 1.0, 1.0],
            smoothness: 0.0,
            params: [1.0, 0.0, 0.0, 0.0],
        }],
        leaf_aabbs: vec![LeafAabb {
            aabb_min: [-1.0, -1.0, -1.0],
            flags: IS_RAYMARCH | ROLE_RAYMARCH_ADD,
            aabb_max: [1.0, 1.0, 1.0],
            entity_id: 0,
        }],
        max_smoothness_radius: 0.0,
    };
    let chunk_id = ChunkId::new(glam::IVec3::new(0, 0, 0), 0);
    renderer
        .bvh_state_mut()
        .insert_streaming_chunk(&queue, chunk_id, &content)
        .expect("insert sphere chunk into renderer pool");

    // Drive the per-frame TLAS-rebuild + uniforms upload via the same
    // entry point the production pipeline calls (no test backdoor —
    // this is the function `update_scene` invokes after every pool
    // mutation lands).
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("raymarch_smoke_setup_encoder"),
    });
    renderer
        .bvh_state_mut()
        .tick_uniforms(&queue, &mut encoder, 0.0, 0.0);
    queue.submit(std::iter::once(encoder.finish()));

    // PR-4 of epic #370: production raymarch reads the GDF cascade-0
    // at every step instead of descending the TLAS. Populate it here
    // so the sphere actually shows up — without the dispatch the
    // texture is zero everywhere and the `eval_scene_bvh` cascade
    // fetch returns SDF=0 at every voxel, making every ray hit
    // immediately at t=0 with NaN normals (the test would still
    // pass on radv via implementation-specific NaN clamping but
    // would silently regress on any conformant backend).
    // --- Camera + scene_meta uniforms (mirror update_camera) ---------
    // Camera at (0, 0, 5) looking at origin, FOV 60°, aspect 1, near
    // 0.1, far 100. Camera-OUTSIDE-AABB on purpose — this is the
    // configuration that exposed the original point-query bug.
    let camera_pos = Vec3::new(0.0, 0.0, 5.0);
    renderer.dispatch_gdf_populate(&device, &queue, camera_pos);
    let view = Mat4::look_at_rh(camera_pos, Vec3::ZERO, Vec3::Y);
    let projection = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let cam = CameraUniforms {
        view: view.to_cols_array_2d(),
        projection: projection.to_cols_array_2d(),
        inverse_view: view.inverse().to_cols_array_2d(),
        inverse_projection: projection.inverse().to_cols_array_2d(),
        position: camera_pos.to_array(),
        _pad0: 0.0,
    };
    renderer.write_camera_uniforms(&queue, bytemuck::bytes_of(&cam));

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
    renderer.write_scene_meta(&queue, bytemuck::bytes_of(&meta));

    // --- Offscreen color + depth -------------------------------------
    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("raymarch_smoke_color"),
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
        label: Some("raymarch_smoke_depth"),
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

    // wgpu requires bytes_per_row aligned to COPY_BYTES_PER_ROW_ALIGNMENT
    // (256). 64 px × 4 B/px = 256 B exactly.
    let bytes_per_row = TARGET_SIZE * 4;
    assert_eq!(bytes_per_row % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("raymarch_smoke_staging"),
        size: (bytes_per_row * TARGET_SIZE) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    // --- Render + readback ------------------------------------------
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("raymarch_smoke_render_encoder"),
    });
    renderer.render(&mut encoder, &color_view, &depth_view, true);
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
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
    let pixels = slice.get_mapped_range().to_vec();

    // --- Asserts -----------------------------------------------------
    let pixel_at = |x: u32, y: u32| -> [u8; 4] {
        let idx = ((y * bytes_per_row) + x * 4) as usize;
        [pixels[idx], pixels[idx + 1], pixels[idx + 2], pixels[idx + 3]]
    };
    let centre = pixel_at(TARGET_SIZE / 2, TARGET_SIZE / 2);
    let corner = pixel_at(0, 0);

    // The sphere fills roughly the central third of the frame at this
    // FOV and camera distance. The centre pixel MUST be a hit — the
    // shader's diffuse term (sun_dir.y > 0) plus ambient produces
    // base ≈ (0.8, 0.7, 0.6) at the lit cap. Sky is ~ (180, 220, 255)
    // — distinctly bluer than the sphere material. Use the
    // R > B + 8 inequality so the test trips on either the
    // point-query bug (centre comes out blue ≡ sky) or any future
    // shader rewrite that loses the diffuse contribution.
    assert!(
        centre[0] as i32 > centre[2] as i32 + 8,
        "centre pixel must be sphere material (warm), not sky (cool blue) — \
         got rgba={centre:?}; sphere fragment shader is failing to hit the \
         primitive at p=(0, 0, 0) from camera at (0, 0, 5).",
    );
    // Corner pixels are firmly outside the sphere — must be sky-coloured
    // (B > R holds for the sky gradient at every horizon angle here).
    assert!(
        corner[2] as i32 > corner[0] as i32,
        "corner pixel should be sky (cool blue), got rgba={corner:?}",
    );

    // Layout-pin asserts so renderer struct churn surfaces here too.
    // 4 × mat4x4 (256 B) + vec3 + pad (16 B) = 272.
    assert_eq!(std::mem::size_of::<CameraUniforms>(), 272);
    // 8 × u32 (32 B) + 2 × vec4 (32 B) = 64.
    assert_eq!(std::mem::size_of::<SceneMeta>(), 64);
}
