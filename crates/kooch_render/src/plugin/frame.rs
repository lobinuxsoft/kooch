//! What the frame needs before anything is recorded: the surface's size, the depth target, the
//! assets the meshlet stage reads, and which camera the scene is seen through (#392).

use kooch_core::gpu::{GpuContext, TargetPool};
use kooch_core::resource::Resources;

use super::GameDepth;
use crate::camera_stack::CameraStack;
use crate::meshlet::MeshletRenderStage;

/// What the rest of the frame reads, resolved once so two systems cannot disagree about the camera
/// or the aspect they drew with.
#[derive(Clone, Default)]
pub(super) struct FrameSetup {
    pub aspect: f32,
    /// The base camera and the overlays over it (#1221), read once: the scene pass and the composite
    /// walk the same list in the same order.
    pub stack: CameraStack,
}

pub(super) fn prepare_frame_system(resources: &mut Resources) {
    if resources.get::<MeshletRenderStage>().is_none() {
        super::init_renderers(resources);
    }
    let Some(gpu) = resources.remove::<GpuContext>() else {
        return;
    };
    let (w, h) = gpu.size();

    let mut pool = resources
        .remove::<TargetPool>()
        .unwrap_or_else(|| TargetPool::new(gpu.device()));
    let mut depth = resources
        .remove::<GameDepth>()
        .unwrap_or_else(|| GameDepth::new(&mut pool, (w, h)));
    depth.ensure(&mut pool, (w, h));
    resources.insert(depth);
    resources.insert(pool);

    if let Some(mut stage) = resources.remove::<MeshletRenderStage>() {
        stage.resize(gpu.device(), (w, h));
        stage.sync_assets_to_gpu(gpu.device(), gpu.queue(), resources);
        resources.insert(stage);
    }

    resources.insert(FrameSetup {
        aspect: w as f32 / h.max(1) as f32,
        // Nothing excluded: a game's cameras are all the game's. The editor keeps its own out
        // where it reads the stack, which is its own panel.
        stack: CameraStack::read::<()>(resources),
    });
    resources.insert(gpu);
}
