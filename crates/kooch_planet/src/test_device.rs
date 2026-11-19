//! A device for this crate's GPU tests.

/// A device with the `r16float` storage atlas, or `None` so the test skips without a GPU.
pub fn try_acquire() -> Option<(wgpu::Device, wgpu::Queue)> {
    let adapter = kooch_core::gpu::test_adapter()?;
    let required = wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
    if !adapter.features().contains(required) {
        eprintln!("skipping planet GPU test: adapter missing {required:?}");
        return None;
    }
    pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("kooch_planet::test_device"),
        required_features: required,
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::default(),
    }))
    .ok()
}
