use crate::resource::Resources;

use super::any_system::AnySystem;

/// Runs a batch of consecutive GPU systems with one encoder submission.
pub(super) fn run_gpu_batch(systems: &mut [AnySystem], resources: &mut Resources) {
    use crate::gpu::GpuContext;

    use crate::gpu::TargetPool;

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
        if let Some(gpu_sys) = sys.as_gpu() {
            if !gpu_sys.is_initialized() {
                gpu_sys.init(gpu.device(), gpu.queue());
                tracing::debug!(system = gpu_sys.name(), "GPU system initialized");
            }
            gpu_sys.prepare(gpu.device(), gpu.queue(), resources);
        }
    }

    // The pool a pass draws into. Absent until a renderer built one, and a batch that has to make
    // its own leaves it behind for the next.
    let mut targets = resources
        .remove::<TargetPool>()
        .unwrap_or_else(|| TargetPool::new(gpu.device()));

    // Recording phase — one encoder, and each system opens the passes it needs.
    let mut encoder = gpu
        .device()
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu_system_encoder"),
        });

    for sys in systems.iter_mut() {
        // The batch shares one encoder, so a GPU system has no `run` of
        // its own to wrap — the scope covers what it records instead.
        let _scope = sys.scope();
        if let Some(gpu_sys) = sys.as_gpu() {
            // What the pass label used to carry: a system may open none, one or several passes, so
            // the name belongs around the recording rather than on any one of them.
            encoder.push_debug_group(gpu_sys.name());
            let frame = crate::system::Frame {
                device: gpu.device(),
                queue: gpu.queue(),
                targets: &mut targets,
            };
            gpu_sys.record(frame, &mut encoder);
            encoder.pop_debug_group();
        }
    }

    gpu.queue().submit(std::iter::once(encoder.finish()));

    resources.insert(targets);
    // Restore GpuContext.
    resources.insert(gpu);
}
