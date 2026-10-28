//! One adapter per test binary.

use std::sync::OnceLock;

/// The adapter every GPU test asks its device from, or `None` without a GPU.
///
/// Shared because creating instances from several test threads at once crashes the Vulkan loader
/// while it scans the ICDs, when the NVIDIA one is installed on an AMD machine (#334). Each test
/// still requests its own device, so its features and limits stay its own.
pub fn test_adapter() -> Option<wgpu::Adapter> {
    static ADAPTER: OnceLock<Option<wgpu::Adapter>> = OnceLock::new();
    ADAPTER
        .get_or_init(|| {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
                flags: wgpu::InstanceFlags::default(),
                backend_options: wgpu::BackendOptions::default(),
                memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
                display: None,
            });
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            }))
            .ok()
        })
        .clone()
}
