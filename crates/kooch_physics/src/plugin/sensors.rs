//! Who is inside which sensor, kept between the solver's arrival and departure (#1222).
//!
//! The solver reports two frames — began touching, stopped touching. A region that does something
//! *while* a body is in it needs the frames in between, and a region that does it gradually needs
//! to know how far in. Both land in [`SensorOccupancy`], which is plain data: a crate that must not
//! depend on the solver can still read it.

use glam::Vec3;
use kooch_core::event::Events;
use kooch_core::resource::Resources;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::entity::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::sensor_occupancy::SensorOccupancy;

use crate::components::{Collider, SHAPE_CAPSULE, SHAPE_CUBOID, SHAPE_SPHERE};

use super::events::{CollisionStarted, CollisionStopped};

/// Records arrivals and departures, then re-measures how deep everything still inside is.
pub(super) fn sensor_occupancy_system(resources: &mut Resources) {
    let mut inside = resources
        .remove::<SensorOccupancy>()
        .unwrap_or_else(SensorOccupancy::default);

    let arrivals: Vec<(Entity, Entity)> = resources
        .get::<Events<CollisionStarted>>()
        .map(|events| {
            events
                .read()
                .filter(|event| event.sensor)
                .map(|event| (event.a, event.b))
                .collect()
        })
        .unwrap_or_default();
    let departures: Vec<(Entity, Entity)> = resources
        .get::<Events<CollisionStopped>>()
        .map(|events| {
            events
                .read()
                .filter(|event| event.sensor)
                .map(|event| (event.a, event.b))
                .collect()
        })
        .unwrap_or_default();

    let Some(registry) = resources.get::<ComponentRegistry>() else {
        resources.insert(inside);
        return;
    };
    let is_sensor = |entity: Entity| {
        registry
            .get_cpu::<Collider>()
            .and_then(|colliders| colliders.get(entity))
            .is_some_and(|collider| collider.sensor)
    };
    // The report does not say which side is the sensor, and both can be: a pair of sensors
    // overlapping is two regions, each holding the other.
    for (a, b) in arrivals {
        for (sensor, body) in [(a, b), (b, a)] {
            if is_sensor(sensor) {
                inside.enter(sensor, body, 0.0);
            }
        }
    }
    for (a, b) in departures {
        inside.leave(a, b);
        inside.leave(b, a);
    }

    // Re-measured every frame, because a body inside a region moves and so does the region.
    let measured: Vec<(Entity, Entity, f32)> = inside
        .iter()
        .map(|occupant| {
            let depth = depth_of(registry, occupant.sensor, occupant.body);
            (occupant.sensor, occupant.body, depth)
        })
        .collect();
    for (sensor, body, depth) in measured {
        inside.enter(sensor, body, depth);
    }

    resources.insert(inside);
}

/// How far `body`'s origin sits past `sensor`'s surface, in metres. Negative once it is out — a
/// departure lands a frame later than the crossing, and a weight read in between must not claim it
/// is still inside.
fn depth_of(registry: &ComponentRegistry, sensor: Entity, body: Entity) -> f32 {
    let Some(collider) = registry
        .get_cpu::<Collider>()
        .and_then(|colliders| colliders.get(sensor))
    else {
        return f32::INFINITY;
    };
    let transforms = registry.get_cpu::<GlobalTransform>();
    let Some((region, at)) =
        transforms.and_then(|transforms| Some((transforms.get(sensor)?, transforms.get(body)?)))
    else {
        return f32::INFINITY;
    };
    let (scale, rotation, translation) = region.matrix.to_scale_rotation_translation();
    // The body's origin in the region's own space, undoing the scale so the extents below are the
    // ones the author typed.
    let local =
        (rotation.inverse() * (at.translation() - translation)) / scale.max(Vec3::splat(1e-6));
    match collider.shape {
        SHAPE_SPHERE => collider.radius - local.length(),
        SHAPE_CUBOID => {
            let gap = collider.half_extents - local.abs();
            gap.x.min(gap.y).min(gap.z)
        }
        SHAPE_CAPSULE => {
            // Distance to the segment down Y, which is the capsule without its caps.
            let y = local.y.clamp(-collider.half_height, collider.half_height);
            collider.radius - (local - Vec3::new(0.0, y, 0.0)).length()
        }
        // A hull, a trimesh, a voxel field: the solver says a body is inside and nothing cheap says
        // how far. All of it, from the moment it arrives.
        _ => f32::INFINITY,
    }
}

#[cfg(test)]
mod tests;
