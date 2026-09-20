//! Running a scene's post-process stack over a colour target (#1201) — shared by the editor's
//! viewports and the game window, so both show the same frame.

use kooch_core::Guid;
use kooch_core::gpu::TargetPool;
use kooch_core::resource::Resources;
use kooch_ecs::post_process::PostProcess;
use kooch_ecs::query::Query;

use super::{PostFrame, PostPass};
use crate::material::MaterialPipeline;

/// The colour a stack reads and writes back into.
pub struct StackTarget<'a> {
    pub texture: &'a wgpu::Texture,
    pub view: &'a wgpu::TextureView,
    pub size: (u32, u32),
    pub format: wgpu::TextureFormat,
}

/// The frame's stack: the scene's [`PostProcess`] with every reached [`PostProcessVolume`] folded
/// over it in priority order (#1222). An effect that is off, weightless or empty is dropped here,
/// so it costs nothing.
pub fn active_stack(resources: &Resources) -> Vec<(Guid, f32)> {
    let base = scene_stack(resources);
    let reached = super::volumes::reached(resources);
    match reached.is_empty() {
        true => base,
        false => super::volumes::folded(&base, &reached),
    }
}

/// The layer underneath every volume: the look with nobody anywhere.
fn scene_stack(resources: &Resources) -> Vec<(Guid, f32)> {
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

/// Runs `stack` over `target`, first to last: each effect reads what the one before it wrote.
pub fn run_stack(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    target: StackTarget<'_>,
    stack: &[(Guid, f32)],
    resources: &mut Resources,
) {
    if stack.is_empty() {
        return;
    }
    // One pass per colour format: its pipelines are built for it.
    let mut pass = resources
        .remove::<PostPass>()
        .filter(|pass| pass.format() == target.format)
        .unwrap_or_else(|| PostPass::new(device, target.format));
    let mut pool = resources
        .remove::<TargetPool>()
        .unwrap_or_else(|| TargetPool::new(device));
    let time = resources
        .get::<kooch_core::time::Time>()
        .map(|time| time.elapsed_secs())
        .unwrap_or(0.0);

    if let Some(materials) = resources.get::<MaterialPipeline>() {
        for &(material, weight) in stack {
            pass.apply(
                PostFrame {
                    device,
                    queue,
                    encoder,
                    targets: &mut pool,
                    scene: target.texture,
                    scene_view: target.view,
                    size: target.size,
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
