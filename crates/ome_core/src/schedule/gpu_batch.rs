use crate::resource::Resources;

use super::any_system::AnySystem;

/// Runs a batch of consecutive GPU systems with one encoder submission.
pub(super) fn run_gpu_batch(systems: &mut [AnySystem], resources: &mut Resources) {
    use crate::gpu::GpuContext;

    let Some(gpu) = resources.remove::<GpuContext>() else {
        let names: Vec<&str> = systems.iter().map(|s| s.name()).collect();
        tracing::warn!(
            systems = ?names,
            "GpuContext not available, skipping GPU systems",
        );
        return;
    };

    // Init + prepare phase (GpuContext removed from resources).
    for sys in systems.iter_mut() {
        if let AnySystem::Gpu(gpu_sys) = sys {
            if !gpu_sys.is_initialized() {
                gpu_sys.init(gpu.device(), gpu.queue());
                tracing::debug!(system = gpu_sys.name(), "GPU system initialized");
            }
            gpu_sys.prepare(gpu.device(), gpu.queue(), resources);
        }
    }

    // Dispatch phase — one encoder, one pass per system.
    let mut encoder =
        gpu.device()
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("gpu_system_encoder"),
            });

    for sys in systems.iter() {
        if let AnySystem::Gpu(gpu_sys) = sys {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(gpu_sys.name()),
                timestamp_writes: None,
            });
            gpu_sys.dispatch(&mut pass);
        }
    }

    gpu.queue().submit(std::iter::once(encoder.finish()));

    // Restore GpuContext.
    resources.insert(gpu);
}
