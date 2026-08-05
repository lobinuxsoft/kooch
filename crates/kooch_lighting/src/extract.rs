//! The ECS walk: light components + their world transforms → the flat
//! record the shader loops over.
//!
//! Pure and `Resources`-only, so the whole extraction is testable with
//! no GPU in the room. The buffer upload is [`crate::GpuLights`]'s job.

use kooch_core::resource::Resources;
use kooch_ecs::directional_light::DirectionalLight;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::point_light::PointLight;
use kooch_ecs::query::Query;
use kooch_ecs::spot_light::SpotLight;

use crate::gpu_light::GpuLight;

/// Light count past which the shader's linear loop stops being an
/// honest implementation and starts being a performance bug. Warned
/// about, never enforced: clipping a scene's lights silently is worse
/// than rendering it slowly, and the caller can see the log.
///
/// The fix is clustering, deliberately out of scope for #441 — Bevy
/// moved theirs to the GPU in 0.18/0.19 and measured ~20× on their
/// `many_lights` benchmark. A universe has stars.
const LINEAR_LOOP_BUDGET: usize = 256;

/// Walks the world for every active light and packs it for the GPU.
///
/// A light with no `GlobalTransform` is skipped rather than defaulted:
/// a directional light without a transform has no direction, and
/// placing it at the origin pointing down would be an invention.
pub fn extract_lights(resources: &Resources) -> Vec<GpuLight> {
    let mut out = Vec::new();

    Query::<(&DirectionalLight, &GlobalTransform)>::new(resources).for_each(
        |(light, transform)| {
            if light.active {
                out.push(GpuLight::directional(light, transform.matrix));
            }
        },
    );
    Query::<(&PointLight, &GlobalTransform)>::new(resources).for_each(|(light, transform)| {
        if light.active {
            out.push(GpuLight::point(light, transform.matrix));
        }
    });
    Query::<(&SpotLight, &GlobalTransform)>::new(resources).for_each(|(light, transform)| {
        if light.active {
            out.push(GpuLight::spot(light, transform.matrix));
        }
    });

    out
}

/// `Some(count)` when `lights` is past the budget the shader's linear
/// loop can carry honestly, `None` otherwise.
///
/// Separate from the walk because the walk runs once **per view per
/// frame** — warning inside it would emit sixty lines a second per open
/// panel, and the advice would be the thing making the log unreadable.
/// [`crate::GpuLights::update`] warns on the transition instead.
pub(crate) fn over_linear_budget(lights: usize) -> Option<usize> {
    (lights > LINEAR_LOOP_BUDGET).then_some(LINEAR_LOOP_BUDGET)
}

// Tests live in `tests/extraction.rs`: exercising the walk needs a
// real `ComponentRegistry` + `ArchetypeRegistry`, which is an
// integration-shaped fixture, not a unit-shaped one.
