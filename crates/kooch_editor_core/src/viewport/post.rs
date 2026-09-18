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

/// Runs the scene's post-process over `target`. A no-op without the component, without a material,
/// or with a material whose shader is not a post-process.
pub(crate) fn apply(
    gpu: &GpuContext,
    encoder: &mut wgpu::CommandEncoder,
    target: &ViewportTarget,
    resources: &mut Resources,
) {
    let Some(material) = active_material(resources) else {
        return;
    };
    let mut pass = resources
        .remove::<PostPass>()
        .unwrap_or_else(|| PostPass::new(gpu.device(), target.format()));
    let mut pool = resources.remove::<TargetPool>().unwrap_or_default();
    let time = resources
        .get::<kooch_core::time::Time>()
        .map(|time| time.elapsed_secs())
        .unwrap_or(0.0);

    if let Some(materials) = resources.get::<MaterialPipeline>() {
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
            },
            materials,
            material,
        );
    }

    resources.insert(pool);
    resources.insert(pass);
}

/// The material of the first enabled [`PostProcess`] in the scene.
fn active_material(resources: &Resources) -> Option<Guid> {
    let mut found = None;
    Query::<&PostProcess>::new(resources).for_each(|post| {
        if found.is_none() && post.enabled {
            found = post.material;
        }
    });
    found
}

/// Why the post-process shader did not compile, for a panel to show.
pub(crate) fn refusal(resources: &Resources) -> Option<String> {
    resources
        .get::<PostPass>()
        .and_then(|pass| pass.refusal().map(str::to_owned))
}
