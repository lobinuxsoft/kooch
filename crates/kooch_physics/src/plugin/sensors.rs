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
use crate::plugin::world::SolverBody;

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
    // 🔴 The solver's answer, not the component's. The sync authors colliders nobody typed — a
    // volume is a sensor whether or not the box is ticked — and asking the component would drop
    // exactly the overlaps this exists for.
    let world = resources.get::<crate::plugin::world::PhysicsWorld>();
    let is_sensor = |entity: Entity| {
        let Some(world) = world else {
            return false;
        };
        registry
            .get_cpu::<SolverBody>()
            .and_then(|slots| slots.get(entity))
            .and_then(|body| world.spec(body.slot()))
            .is_some_and(|spec| spec.is_sensor())
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

/// The same answer without a solver (#1222). The editor mirrors a project's components and runs no
/// physics, so nothing fills [`SensorOccupancy`] there — and the Game panel is exactly where an
/// author looks to see whether a region works.
///
/// 🔴 Origins, where the solver overlaps whole shapes: a preview that is a body's width optimistic
/// at the boundary, and the same answer everywhere else. Registered by the components-only plugin,
/// so a host with a solver never runs it.
pub(super) fn sensor_occupancy_preview_system(resources: &mut Resources) {
    // A host with a solver has the real answer, from whole shapes and with the arrivals the solver
    // reports. Two producers for one resource would fight every frame.
    if resources
        .get::<crate::plugin::world::PhysicsWorld>()
        .is_some()
    {
        return;
    }
    let mut inside = resources
        .remove::<SensorOccupancy>()
        .unwrap_or_else(SensorOccupancy::default);
    inside.clear();
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        resources.insert(inside);
        return;
    };
    let (Some(volumes), Some(colliders)) = (
        registry.get_cpu::<kooch_ecs::post_process_volume::PostProcessVolume>(),
        registry.get_cpu::<Collider>(),
    ) else {
        resources.insert(inside);
        return;
    };
    for (&region, volume) in volumes.iter() {
        let Some(shape) = colliders
            .get(region)
            .filter(|_| !volume.global && volume.enabled)
        else {
            continue;
        };
        for (&body, collider) in colliders.iter() {
            // A region does not contain itself, and one sensor inside another says nothing about
            // where the game is.
            if body == region || collider.sensor {
                continue;
            }
            if !shape
                .interaction()
                .collision_groups
                .interacts_with(collider.interaction().collision_groups)
            {
                continue;
            }
            let depth = depth_of(registry, region, body);
            if depth >= 0.0 {
                inside.enter(region, body, depth);
            }
        }
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
    let scale = scale.abs();
    // 🔴 Metres, not shape units. The blend distance an author types is a distance in the world, so
    // the extents are scaled up to meet it rather than the position scaled down to meet them: a
    // sphere of radius 1 scaled by five is five metres of region, and dividing instead made every
    // blend five times too fast.
    //
    // The shape sits at its own centre, which the gizmo also draws at: a region offset from its
    // entity would otherwise be measured from the entity.
    let centre = translation + rotation * (collider.center * scale);
    let local = rotation.inverse() * (at.translation() - centre);
    // Radius follows the horizontal axes on everything aligned to Y, the same rule the collider
    // gizmo draws by — what an author sees is what is measured.
    let flat = collider.radius * scale.x.max(scale.z);
    match collider.shape {
        SHAPE_SPHERE => collider.radius * scale.max_element() - local.length(),
        SHAPE_CUBOID => {
            let gap = collider.half_extents * scale - local.abs();
            gap.x.min(gap.y).min(gap.z)
        }
        SHAPE_CAPSULE => {
            // Distance to the segment down Y, which is the capsule without its caps.
            let half_height = collider.half_height * scale.y;
            let y = local.y.clamp(-half_height, half_height);
            flat - (local - Vec3::new(0.0, y, 0.0)).length()
        }
        // A hull, a trimesh, a voxel field: the solver says a body is inside and nothing cheap says
        // how far. All of it, from the moment it arrives.
        _ => f32::INFINITY,
    }
}

#[cfg(test)]
mod tests;
