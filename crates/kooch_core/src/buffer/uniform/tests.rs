use super::*;

fn create_headless_device() -> (Device, Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .expect("no GPU adapter");

    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("test_device"),
        ..Default::default()
    }))
    .expect("failed to create device")
}

/// A simple struct to test encase serialization.
/// f32 min_size = 4 bytes, but std140 may pad it.
#[derive(ShaderType)]
struct TestUniform {
    value: f32,
}

#[test]
#[ignore] // Requires GPU hardware.
fn uniform_write_does_not_panic() {
    let (device, queue) = create_headless_device();

    let mut uniform = UniformBuffer::<TestUniform>::new(&device, "test_uniform");
    uniform.write(&queue, &TestUniform { value: 42.0 });

    // Write again to verify scratch reuse.
    uniform.write(&queue, &TestUniform { value: 99.0 });
}

/// Multi-field struct to test padding and layout.
#[derive(ShaderType)]
struct TestVec4 {
    x: f32,
    y: f32,
    z: f32,
    w: f32,
}

#[test]
#[ignore] // Requires GPU hardware.
fn uniform_multi_field() {
    let (device, queue) = create_headless_device();

    let mut uniform = UniformBuffer::<TestVec4>::new(&device, "test_vec4");
    uniform.write(
        &queue,
        &TestVec4 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
            w: 4.0,
        },
    );
}
