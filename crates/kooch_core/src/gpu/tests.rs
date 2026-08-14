use wgpu::{Instance, InstanceDescriptor, RequestAdapterOptions};

// Display/Debug/Error::source impls for `GpuError` are exhaustive
// match arms over wgpu-typed payloads, so their correctness is
// proven by compilation. Synthetic-instance tests removed when wgpu
// 29 made `RequestAdapterError` non-constructible from user code
// (issue #218). The headless smoke test below still exercises the
// happy path with a real adapter.

#[test]
#[ignore] // Requires GPU hardware.
fn create_adapter_headless() {
    let instance = Instance::new(InstanceDescriptor {
        backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions::default(),
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });

    let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }));

    assert!(adapter.is_ok(), "expected a GPU adapter to be available");

    let info = adapter.unwrap().get_info();
    println!("Adapter: {} ({:?})", info.name, info.backend);
}

mod frame_latency {
    use crate::gpu::context::latency_from;

    /// Two images is what the surface has had since it was written, and
    /// an unset variable must not quietly change a measurement run.
    #[test]
    fn unset_keeps_two() {
        assert_eq!(latency_from(None), 2);
    }

    /// The value #814 exists to measure.
    #[test]
    fn a_value_is_honoured() {
        assert_eq!(latency_from(Some("3")), 3);
        assert_eq!(latency_from(Some(" 1 ")), 1);
    }

    /// A swapchain cannot hold zero images or eight, and a clamp beats a
    /// panic that would take the window down mid-capture.
    #[test]
    fn out_of_range_is_clamped() {
        assert_eq!(latency_from(Some("0")), 1);
        assert_eq!(latency_from(Some("8")), 3);
    }

    /// 🔴 A typo falls back rather than guessing. Silently measuring a
    /// different swapchain than the one asked for is the failure that
    /// wastes a hardware run.
    #[test]
    fn a_typo_falls_back() {
        assert_eq!(latency_from(Some("tres")), 2);
        assert_eq!(latency_from(Some("")), 2);
    }
}
