//! Volumes over the scene's stack (#1222): where a body is decides what the frame looks like.
//!
//! A [`PostProcessVolume`] contributes its effects while something is inside its sensor, weighted by
//! how far in. The scene's [`PostProcess`](kooch_ecs::post_process::PostProcess) is the layer
//! underneath — the look with nobody anywhere — and the volumes are applied over it in priority
//! order, each overriding what the ones below left by its own weight.

use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::post_process_volume::PostProcessVolume;
use kooch_ecs::query::Query;
use kooch_ecs::sensor_occupancy::SensorOccupancy;

/// One volume that has arrived, and how much of it applies. Owned: a handful of volumes cloned once
/// a frame is cheaper than holding the ECS borrow across the fold.
pub(super) struct Reached {
    pub weight: f32,
    pub volume: PostProcessVolume,
}

/// The volumes contributing this frame, in the order they are applied: priority ascending, so the
/// highest lands last and overrides the rest.
pub(super) fn reached(resources: &Resources) -> Vec<Reached> {
    let occupancy = resources.get::<SensorOccupancy>();
    let mut found: Vec<(i32, u32, Reached)> = Vec::new();
    Query::<&PostProcessVolume>::new(resources).for_each_entity(|entity, volume| {
        // A global volume is everywhere, so it is asked at a depth nothing can be short of; a
        // shaped one is asked at the depth its sensor reports, and -1 means nobody is inside.
        let depth = match volume.global {
            true => f32::INFINITY,
            false => occupancy
                .and_then(|occupancy| occupancy.depth_in(entity))
                .unwrap_or(-1.0),
        };
        let weight = volume.weight_at(depth);
        if weight > 0.0 {
            found.push((
                volume.priority,
                entity.index(),
                Reached {
                    weight,
                    volume: volume.clone(),
                },
            ));
        }
    });
    found.sort_by_key(|(priority, index, _)| (*priority, *index));
    found.into_iter().map(|(_, _, reached)| reached).collect()
}

/// `base` with every volume folded over it. A volume overrides an effect the layers below already
/// have — by its own weight, so arriving is a fade rather than a switch — and adds the ones they do
/// not, which fade in from nothing for the same reason.
pub(super) fn folded(base: &[(Guid, f32)], volumes: &[Reached]) -> Vec<(Guid, f32)> {
    let mut stack: Vec<(Guid, f32)> = base.to_vec();
    for reached in volumes {
        for effect in &reached.volume.effects {
            let Some(material) = effect.enabled.then_some(effect.material).flatten() else {
                continue;
            };
            let wanted = effect.weight.clamp(0.0, 1.0);
            match stack.iter_mut().find(|(guid, _)| *guid == material) {
                // Unity's model: what the volume asks for, reached in proportion to how much of the
                // volume applies. Leaving it ramps back to what was underneath.
                Some((_, weight)) => *weight += (wanted - *weight) * reached.weight,
                None => stack.push((material, wanted * reached.weight)),
            }
        }
    }
    // An effect that ended at nothing is not drawn at all, which is the whole point of a weight
    // that reaches zero when you walk out.
    stack.retain(|(_, weight)| *weight > 0.0);
    stack
}

#[cfg(test)]
mod tests;
