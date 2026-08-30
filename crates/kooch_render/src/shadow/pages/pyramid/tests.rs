use super::*;

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        ..Default::default()
    }))
    .ok()
}

/// A table with exactly one resident page, at `(level, x, y)`.
fn table_with(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    side: u32,
    levels: u32,
    at: (u32, u32, u32),
) -> wgpu::Buffer {
    let entries = (side * side * levels) as usize;
    let mut words = vec![0u32; entries * 6];
    let page = at.0 * side * side + at.2 * side + at.1;
    // Entries store `slot + 1`, so any non-zero word is resident.
    words[page as usize * 6] = 1;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (words.len() * 4) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&words));
    buffer
}

/// One mip of one layer, read back with the row padding wgpu requires.
fn read_mip(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip: u32,
    layer: u32,
    side: u32,
) -> Vec<u32> {
    let row = (side * 4).div_ceil(256) * 256;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * side) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: mip,
            origin: wgpu::Origin3d {
                x: 0,
                y: 0,
                z: layer,
            },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(side),
            },
        },
        wgpu::Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    let raw = staging.slice(..).get_mapped_range().to_vec();
    let mut out = Vec::with_capacity((side * side) as usize);
    for y in 0..side {
        let start = (y * row) as usize;
        out.extend_from_slice(bytemuck::cast_slice::<u8, u32>(
            &raw[start..start + (side * 4) as usize],
        ));
    }
    out
}

/// One resident page has to light every texel above it, and only those.
///
/// 🔴 Both halves matter and they fail differently. A missing ancestor
/// is a caster the overlap test will reject — geometry that silently
/// stops being drawn into a page that asked for it, which is the exact
/// shape of the artefact this whole line of work is chasing. A spurious
/// one is only wasted raster. The first is why the structure has to be
/// proven before anything reads it.
#[test]
fn a_resident_page_lights_its_ancestors() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter; skipping");
        return;
    };
    // Small enough to read back whole, still a real chain of five mips.
    let config = PageConfig {
        page: 128,
        virtual_size: 128 * 16,
        ..PageConfig::default()
    };
    let clipmap = ClipmapConfig {
        base: 1.28,
        levels: 3,
    };
    let side = config.side(0);
    assert_eq!(side, 16, "the fixture wanted a 16x16 grid");

    let pyramid = PagePyramid::new(&device, config, clipmap);
    let at = (1u32, 5u32, 9u32);
    let table = table_with(&device, &queue, side, clipmap.levels, at);

    let mut encoder = device.create_command_encoder(&Default::default());
    pyramid.build(&device, &queue, &mut encoder, &table, 0);
    queue.submit([encoder.finish()]);

    for mip in 0..PagePyramid::mip_count(side) {
        let mip_side = (side >> mip).max(1);
        let texels = read_mip(&device, &queue, pyramid.texture(), mip, at.0, mip_side);
        let want = ((at.2 >> mip) * mip_side + (at.1 >> mip)) as usize;
        assert_eq!(
            texels[want], 1,
            "mip {mip} lost the page at ({}, {}) — an ancestor that reads 0 is a caster the \
             overlap test rejects, and the geometry stops being drawn with nothing failing",
            at.1, at.2
        );
        let lit = texels.iter().filter(|&&t| t != 0).count();
        assert_eq!(
            lit, 1,
            "mip {mip} lit {lit} texels for one resident page — wasted raster, and it means \
             the reduction read outside its own block",
        );
    }

    // A level nobody marked stays dark, or the pyramid is answering for
    // the wrong clipmap level entirely.
    let other = read_mip(&device, &queue, pyramid.texture(), 0, 0, side);
    assert!(
        other.iter().all(|&t| t == 0),
        "level 0 lit up for a page marked on level 1",
    );
}
