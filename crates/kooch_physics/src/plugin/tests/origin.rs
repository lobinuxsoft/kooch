//! A world far from zero: the solver works relative to the world origin, `Transform` stays exact.

use glam::{DVec3, IVec3};
use kooch_ecs::hierarchy::{OriginShift, WorldOrigin};

use super::*;
use crate::plugin::interpolation::follow_origin;

const AT: DVec3 = DVec3::new(0.0, 6_371_010.0, 0.0);

fn exact(resources: &Resources, entity: Entity) -> DVec3 {
    resources
        .get::<ComponentRegistry>()
        .and_then(|r| r.get_cpu::<Transform>())
        .and_then(|s| s.get(entity))
        .map(|t| t.position)
        .expect("entity has no Transform")
}

/// Spawned since the last propagation, so the solver reads its `Transform`, not a global.
fn far_body(resources: &mut Resources) -> Entity {
    resources.insert(WorldOrigin {
        cell: IVec3::new(0, 3111, 0),
    });
    spawn_body(
        resources,
        Transform::from_position(AT),
        PhysicsBody::default(),
        Collider::default(),
    )
}

/// Without the origin, a fresh body entered the solver 6 371 km out and wrote that back.
#[test]
fn a_far_spawn_keeps_its_place() {
    let mut resources = world();
    let body = far_body(&mut resources);
    physics_sync_system(&mut resources);
    physics_writeback_system(&mut resources);
    assert_eq!(exact(&resources, body), AT);
}

/// A rebase moves the solver with the origin: the authored pose does not move.
#[test]
fn a_rebase_keeps_body_poses() {
    let mut resources = world();
    let body = far_body(&mut resources);
    physics_sync_system(&mut resources);
    resources.insert(WorldOrigin {
        cell: IVec3::new(0, 3112, 0),
    });
    resources.insert(OriginShift {
        cells: IVec3::new(0, 1, 0),
    });
    follow_origin(&mut resources);
    physics_writeback_system(&mut resources);
    assert_eq!(exact(&resources, body), AT);
}
