use super::*;

#[test]
fn the_bins_shader_validates() {
    let module = naga::front::wgsl::parse_str(TILE_BINS_SHADER).expect("parses");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("{}", e.emit_to_string(TILE_BINS_SHADER)));
}

/// The layout constants the shading frame reads the list with.
#[test]
fn the_frame_reads_the_same_layout() {
    let frame = crate::meshlet::MATERIAL_COMPUTE_FRAME;
    assert!(TILE_BINS_SHADER.contains(&format!("const LIST: u32 = {HEADER}u;")));
    assert!(frame.contains(&format!("const BIN_LIST: u32 = {HEADER}u;")));
    assert!(frame.contains("const BIN_ROW: u32 = 4096u;"));
    assert!(TILE_BINS_SHADER.contains("const ROW: u32 = 4096u;"));
}

fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::default();
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .ok()?;
    let features = kooch_core::gpu::vbuf64_features();
    if !adapter.features().contains(features) {
        return None;
    }
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: None,
        required_features: features,
        // The engine asks for the adapter's own; the downlevel default of four is not what it runs.
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .ok()
}

fn read(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<u32> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: buffer.size(),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(30)),
    });
    bytemuck::cast_slice(&staging.slice(..).get_mapped_range()).to_vec()
}

/// 🔴 Two tiles: the left all material 1, the right material 1 with a row of material 2. Material 1
/// dispatches over both tiles, material 2 over the right one only, and the fallback over none.
#[test]
fn each_material_gets_its_tiles() {
    let Some((device, queue)) = device() else {
        eprintln!("no adapter with the vbuf64 features; skipped");
        return;
    };
    let (width, height) = (32u32, 16u32);
    let vbuf = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: VBUF64_FORMAT,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // Covered depth in the high half, `visible_slot << 7` in the low: slot 0 is instance 0
    // (material 1), slot 1 is instance 1 (material 2).
    let texel = |slot: u64| (1u64 << 32) | (slot << 7);
    let mut texels = vec![texel(0); (width * height) as usize];
    for x in 16..width {
        texels[x as usize] = texel(1);
    }
    queue.write_texture(
        vbuf.as_image_copy(),
        bytemuck::cast_slice(&texels),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 8),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let storage = |words: &[u32]| {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (words.len() * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buffer, 0, bytemuck::cast_slice(words));
        buffer
    };
    let visible_meshlets = storage(&[0, 1 << 16]);
    // Two 96-byte instances; `material_id` is the word after the 64-byte transform and `mesh_id`.
    let mut instances = [0u32; 48];
    instances[17] = 1;
    instances[24 + 17] = 2;
    let instances = storage(&instances);

    let bins = TileBins::new(&device);
    let mut encoder = device.create_command_encoder(&Default::default());
    let binned = bins.bin(
        &device,
        &queue,
        &mut encoder,
        &vbuf.create_view(&Default::default()),
        &visible_meshlets,
        &instances,
        (width, height),
        (2, 1),
        1,
        3,
    );
    queue.submit([encoder.finish()]);

    let words = read(&device, &queue, &binned.bins);
    let args = read(&device, &queue, &binned.args);
    assert_eq!(words[0], 0, "no overflow");
    assert_eq!(words[1], 2, "tiles per row");
    let list = |slot: usize| {
        let (first, end) = (words[2 + slot], words[3 + slot]);
        let mut tiles =
            words[(HEADER + first as u64) as usize..(HEADER + end as u64) as usize].to_vec();
        tiles.sort();
        tiles
    };
    assert_eq!(list(0), Vec::<u32>::new());
    assert_eq!(list(1), vec![0, 1]);
    assert_eq!(list(2), vec![1]);
    assert_eq!(&args[0..9], &[0, 1, 1, 2, 1, 1, 1, 1, 1]);
}
