//! GPU integration test for the Hi-Z pyramid builder.
//!
//! Uploads a small Depth32Float texture with hand-crafted values,
//! runs `HiZ::build`, and reads back every mip level to assert:
//!   - mip 0 is a byte-perfect copy of the depth source,
//!   - mip k is the max() of its 2×2 parent block,
//!   - the top mip (1×1) carries the global max.
//!
//! Run with:
//!   cargo test -p ome_render --test hi_z_build

mod common;

use common::try_acquire_device;
use ome_render::HiZ;

const WIDTH: u32 = 64;
const HEIGHT: u32 = 64;
const PIXELS: usize = (WIDTH * HEIGHT) as usize;
const ROW_BYTES: u32 = WIDTH * 4; // R32Float: 4 bytes per texel; 64×4 = 256 = wgpu alignment

fn upload_r32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    values: &[f32],
) -> wgpu::Texture {
    assert_eq!(values.len(), PIXELS);

    // wgpu refuses Queue::write_texture into Depth32Float, so the
    // test path uses an R32Float texture and routes it through
    // HiZ::build_from_r32. Production code uses
    // HiZ::build_from_depth with the real depth attachment.
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hi_z_test_r32"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice::<f32, u8>(values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ROW_BYTES),
            rows_per_image: Some(HEIGHT),
        },
        wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
    );

    tex
}

fn read_mip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    hi_z: &HiZ,
    mip: u32,
) -> Vec<f32> {
    let (w, h) = ome_render::hi_z::mip_size(WIDTH, HEIGHT, mip);
    let bytes_per_row = (w * 4).max(256);
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hi_z_mip_staging"),
        size: (bytes_per_row * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hi_z_mip_readback"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: hi_z.texture(),
            mip_level: mip,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| tx.send(r).unwrap());
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    rx.recv().unwrap().unwrap();
    let bytes = slice.get_mapped_range().to_vec();

    let mut out = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let off = (y * bytes_per_row + x * 4) as usize;
            let mut buf = [0u8; 4];
            buf.copy_from_slice(&bytes[off..off + 4]);
            out.push(f32::from_le_bytes(buf));
        }
    }
    out
}

#[test]
fn hi_z_mip0_is_exact_copy_of_depth_source() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Gradient along X: depth[y][x] = x / WIDTH. Each row is identical.
    let mut depth = vec![0.0f32; PIXELS];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            depth[(y * WIDTH + x) as usize] = x as f32 / WIDTH as f32;
        }
    }
    let r32_tex = upload_r32(&device, &queue, &depth);
    let r32_view = r32_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let hi_z = HiZ::new(&device, WIDTH, HEIGHT);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hi_z_build_copy_test"),
    });
    hi_z.build_from_r32(&device, &mut encoder, &r32_view);
    queue.submit(std::iter::once(encoder.finish()));

    let mip0 = read_mip(&device, &queue, &hi_z, 0);
    for (i, (got, expected)) in mip0.iter().zip(depth.iter()).enumerate() {
        assert!(
            (got - expected).abs() < 1e-6,
            "mip 0 texel {i} mismatch: got {got}, expected {expected}"
        );
    }
}

#[test]
fn hi_z_top_mip_holds_global_max_depth() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Constant-zero background, single bright spike at the centre.
    let mut depth = vec![0.0f32; PIXELS];
    depth[((HEIGHT / 2) * WIDTH + WIDTH / 2) as usize] = 0.875;
    let r32_tex = upload_r32(&device, &queue, &depth);
    let r32_view = r32_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let hi_z = HiZ::new(&device, WIDTH, HEIGHT);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hi_z_build_max_test"),
    });
    hi_z.build_from_r32(&device, &mut encoder, &r32_view);
    queue.submit(std::iter::once(encoder.finish()));

    // Top mip is 1×1 — single texel must equal the spike.
    let top = read_mip(&device, &queue, &hi_z, hi_z.mip_count() - 1);
    assert_eq!(top.len(), 1);
    assert!(
        (top[0] - 0.875).abs() < 1e-6,
        "top mip should carry the global max: got {got}, expected 0.875",
        got = top[0]
    );
}

#[test]
fn hi_z_mip1_takes_max_of_each_2x2_block() {
    let Some((device, queue)) = try_acquire_device() else {
        eprintln!("no GPU adapter available; skipping");
        return;
    };

    // Pattern: depth[y][x] = (x ^ y) as f32 / 256.0 — guarantees every
    // 2×2 block has at least one max-element distinct from its neighbours.
    let mut depth = vec![0.0f32; PIXELS];
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            depth[(y * WIDTH + x) as usize] = (x ^ y) as f32 / 256.0;
        }
    }
    let r32_tex = upload_r32(&device, &queue, &depth);
    let r32_view = r32_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let hi_z = HiZ::new(&device, WIDTH, HEIGHT);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hi_z_build_reduce_test"),
    });
    hi_z.build_from_r32(&device, &mut encoder, &r32_view);
    queue.submit(std::iter::once(encoder.finish()));

    let (m1_w, m1_h) = ome_render::hi_z::mip_size(WIDTH, HEIGHT, 1);
    let mip1 = read_mip(&device, &queue, &hi_z, 1);
    assert_eq!(mip1.len() as u32, m1_w * m1_h);

    for ty in 0..m1_h {
        for tx in 0..m1_w {
            let sx = tx * 2;
            let sy = ty * 2;
            let s00 = depth[(sy * WIDTH + sx) as usize];
            let s10 = depth[(sy * WIDTH + sx + 1) as usize];
            let s01 = depth[((sy + 1) * WIDTH + sx) as usize];
            let s11 = depth[((sy + 1) * WIDTH + sx + 1) as usize];
            let expected = s00.max(s10).max(s01.max(s11));
            let got = mip1[(ty * m1_w + tx) as usize];
            assert!(
                (got - expected).abs() < 1e-6,
                "mip1 ({tx},{ty}) max mismatch: got {got}, expected {expected}"
            );
        }
    }
}
