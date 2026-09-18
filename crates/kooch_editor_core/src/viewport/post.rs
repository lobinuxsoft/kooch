//! The post-process slot in the editor's viewports (#1201).
//!
//! 🔴 It runs after the blit and before the gizmos: the effect belongs to what the camera rendered,
//! and a vignette over the move handles would be a vignette over the editor's own UI.

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_render::post_process::{StackTarget, active_stack, run_stack};

use super::target::ViewportTarget;

/// Runs the scene's post-process stack over `target`. A no-op without the component or with an
/// empty stack.
pub(crate) fn apply(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    target: &ViewportTarget,
    resources: &mut Resources,
) {
    let stack = active_stack(resources);
    run_stack(
        gpu.device(),
        gpu.queue(),
        encoder,
        StackTarget {
            texture: target.color_texture(),
            view: target.view(),
            size: target.size(),
            format: target.format(),
        },
        &stack,
        resources,
    );
}
