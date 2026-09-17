//! What the frame needs before anything is recorded: the surface's size, the depth target, the
//! assets the meshlet stage reads, and which camera the scene is seen through (#392).

use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::perspective_camera::PerspectiveCamera;
use kooch_ecs::query::Query;

use super::GameDepth;
use crate::meshlet::MeshletRenderStage;

/// What the rest of the frame reads, resolved once so two systems cannot disagree about the camera
/// or the aspect they drew with.
#[derive(Clone, Default)]
pub(super) struct FrameSetup {
    pub aspect: f32,
    pub camera: Option<crate::ViewCamera>,
}

pub(super) fn prepare_frame_system(resources: &mut Resources) {
    if resources.get::<MeshletRenderStage>().is_none() {
        super::init_renderers(resources);
    }
    let Some(gpu) = resources.remove::<GpuContext>() else {
        return;
    };
    let (w, h) = gpu.size();

    let mut depth = resources
        .remove::<GameDepth>()
        .unwrap_or_else(|| GameDepth::new(gpu.device(), (w, h)));
    depth.ensure(gpu.device(), (w, h));
    resources.insert(depth);

    if let Some(mut stage) = resources.remove::<MeshletRenderStage>() {
        stage.resize(gpu.device(), (w, h));
        stage.sync_assets_to_gpu(gpu.device(), gpu.queue(), resources);
        resources.insert(stage);
    }

    resources.insert(FrameSetup {
        aspect: w as f32 / h.max(1) as f32,
        camera: active_camera(resources),
    });
    resources.insert(gpu);
}

fn active_camera(resources: &Resources) -> Option<crate::ViewCamera> {
    // Highest-priority active `PerspectiveCamera` wins. Game runtime
    // ties the same way the editor does: priority is the contract,
    // not iteration order.
    let query = Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources);
    let mut best: Option<(i32, crate::ViewCamera)> = None;
    query.for_each(|(cam, gt)| {
        if !cam.active {
            return;
        }
        if let Some((p, _)) = best
            && cam.priority <= p
        {
            return;
        }
        best = Some((cam.priority, crate::ViewCamera::from_components(cam, gt)));
    });
    best.map(|(_, camera)| camera)
}
