//! The scene, recorded and submitted before the swapchain image is asked for (#392). One pass per
//! camera of the stack (#1221): the base into the primary view, each overlay into its own.

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;

use super::frame::FrameSetup;
use crate::camera_stack::StackViews;
use crate::meshlet::MeshletRenderStage;

pub(super) fn render_meshlets_system(resources: &mut Resources) {
    let Some(setup) = resources.get::<FrameSetup>().cloned() else {
        return;
    };
    let Some(gpu) = resources.remove::<GpuContext>() else {
        return;
    };
    let Some(mut stage) = resources.remove::<MeshletRenderStage>() else {
        resources.insert(gpu);
        return;
    };
    let mut views = resources.remove::<StackViews>().unwrap_or_default();

    let stats = stage.render_with_assets_primary(
        gpu.device(),
        gpu.queue(),
        resources,
        // The sky draws only when the scene really has a camera; the meshlet stage falls back to a
        // default lens rather than to an identity matrix, which is not a projection.
        &setup.stack.base.unwrap_or_default(),
        setup.aspect,
    );

    views.retain(&setup.stack, &mut stage);
    let size = gpu.size();
    for (entity, camera) in &setup.stack.overlays {
        let view = views.view_for(*entity, &mut stage, gpu.device(), size);
        let drawn = stage.render_with_assets(
            view,
            gpu.device(),
            gpu.queue(),
            resources,
            camera,
            setup.aspect,
        );
        // The composite reads this: a view that skipped its dispatch still holds the frame before.
        views.mark(*entity, drawn.instances_uploaded > 0);
    }

    // The one measurement a game could not otherwise have: the editor reads these stats, and until
    // now a windowed game threw them away.
    if let Some(metrics) = resources.get_mut::<kooch_core::frame_metrics::FrameMetrics>() {
        metrics.gpu_frame_ms = stats.gpu_frame_ms;
    }

    resources.insert(views);
    resources.insert(stage);
    resources.insert(gpu);
}
