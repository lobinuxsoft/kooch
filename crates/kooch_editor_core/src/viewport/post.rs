//! The post-process slot in the editor's viewports (#1201).
//!
//! 🔴 It runs after the blit and before the gizmos: the effect belongs to what the camera rendered,
//! and a vignette over the move handles would be a vignette over the editor's own UI.

use kooch_core::Guid;
use kooch_core::gpu::{GpuContext, TargetPool};
use kooch_core::resource::Resources;
use kooch_ecs::post_process::PostProcess;
use kooch_ecs::query::Query;
use kooch_render::material::MaterialPipeline;
use kooch_render::post_process::{PostFrame, PostPass};

use super::target::ViewportTarget;

/// Runs the scene's post-process stack over `target`, first to last: each effect reads what the one
/// before it wrote. A no-op without the component or with an empty stack.
pub(crate) fn apply(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    target: &ViewportTarget,
    resources: &mut Resources,
) {
    let stack = active_stack(resources);
    if stack.is_empty() {
        return;
    }
    let mut pass = resources
        .remove::<PostPass>()
        .unwrap_or_else(|| PostPass::new(gpu.device(), target.format()));
    let mut pool = resources
        .remove::<TargetPool>()
        .unwrap_or_else(|| TargetPool::new(gpu.device()));
    let time = resources
        .get::<kooch_core::time::Time>()
        .map(|time| time.elapsed_secs())
        .unwrap_or(0.0);

    if let Some(materials) = resources.get::<MaterialPipeline>() {
        for (material, weight) in stack {
            pass.apply(
                PostFrame {
                    device: gpu.device(),
                    queue: gpu.queue(),
                    encoder,
                    targets: &mut pool,
                    scene: target.color_texture(),
                    scene_view: target.view(),
                    size: target.size(),
                    time,
                    weight,
                },
                materials,
                material,
            );
        }
    }

    resources.insert(pool);
    resources.insert(pass);
}

/// The stack of the first enabled [`PostProcess`] in the scene, with each effect's weight. An
/// effect that is off, weightless or empty is dropped here, so it costs nothing.
fn active_stack(resources: &Resources) -> Vec<(Guid, f32)> {
    let mut found: Option<Vec<(Guid, f32)>> = None;
    Query::<&PostProcess>::new(resources).for_each(|post| {
        if found.is_none() && post.enabled {
            found = Some(
                post.effects
                    .iter()
                    .filter_map(|effect| Some((effect.drawn()?, effect.weight)))
                    .collect(),
            );
        }
    });
    found.unwrap_or_default()
}
