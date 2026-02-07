//! Headless compute shader example — vector addition on the GPU.
//!
//! Demonstrates the full wgpu compute pipeline: create buffers, dispatch a
//! shader that adds two f32 arrays, read back the result, and verify it on CPU.
//!
//! No window or surface is needed — runs entirely headless.

use ome_core::compute::VectorAddCompute;
use wgpu::util::DeviceExt;

const ELEMENT_COUNT: u32 = 1024;

fn main() {
    ome_core::init_tracing();

    // -- Headless wgpu setup (no surface) --
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        ..Default::default()
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter found");

    let info = adapter.get_info();
    println!("Adapter: {} ({:?})", info.name, info.backend);

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("compute_example_device"),
            ..Default::default()
        },
        None,
    ))
    .expect("failed to create device");

    // -- Input data --
    let data_a: Vec<f32> = (0..ELEMENT_COUNT).map(|i| i as f32).collect();
    let data_b: Vec<f32> = (0..ELEMENT_COUNT).map(|i| (i * 2) as f32).collect();

    let buf_size = (ELEMENT_COUNT as usize * size_of::<f32>()) as wgpu::BufferAddress;

    // -- GPU buffers --
    let buf_a = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("input_a"),
        contents: bytemuck::cast_slice(&data_a),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    let buf_b = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("input_b"),
        contents: bytemuck::cast_slice(&data_b),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    });

    let buf_output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("output"),
        size: buf_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });

    let buf_staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: buf_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // -- Dispatch compute shader --
    let compute = VectorAddCompute::new(&device);
    compute.dispatch(&device, &queue, &buf_a, &buf_b, &buf_output, ELEMENT_COUNT);

    // -- Copy output → staging --
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("copy_encoder"),
    });
    encoder.copy_buffer_to_buffer(&buf_output, 0, &buf_staging, 0, buf_size);
    queue.submit(std::iter::once(encoder.finish()));

    // -- Read back to CPU --
    let staging_slice = buf_staging.slice(..);
    staging_slice.map_async(wgpu::MapMode::Read, |result| {
        result.expect("failed to map staging buffer");
    });
    device.poll(wgpu::Maintain::Wait);

    let data = staging_slice.get_mapped_range();
    let result: &[f32] = bytemuck::cast_slice(&data);

    // -- Verify --
    let mut pass = true;
    for i in 0..ELEMENT_COUNT as usize {
        let expected = data_a[i] + data_b[i];
        if (result[i] - expected).abs() > f32::EPSILON {
            eprintln!(
                "MISMATCH at index {i}: expected {expected}, got {}",
                result[i]
            );
            pass = false;
        }
    }

    drop(data);
    buf_staging.unmap();

    if pass {
        println!("PASS: {ELEMENT_COUNT} elements verified correctly.");
        println!(
            "  first 8 results: {:?}",
            &(0..8u32)
                .map(|i| data_a[i as usize] + data_b[i as usize])
                .collect::<Vec<_>>()
        );
    } else {
        eprintln!("FAIL: some elements did not match.");
        std::process::exit(1);
    }
}
