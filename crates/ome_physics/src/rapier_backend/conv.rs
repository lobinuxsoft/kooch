use rapier3d::prelude::*;

use crate::backend::CollisionShape;

/// Builds the Rapier collider for an engine shape.
///
/// Since Rapier 0.32 the math types *are* glam's, so this module no
/// longer translates vectors or quaternions — only shapes, which have no
/// engine-side equivalent.
pub(super) fn collider_for(shape: CollisionShape) -> Collider {
    match shape {
        CollisionShape::Sphere { radius } => ColliderBuilder::ball(radius).build(),
        CollisionShape::Cuboid { half_extents } => ColliderBuilder::cuboid(
            half_extents.x.max(1e-4),
            half_extents.y.max(1e-4),
            half_extents.z.max(1e-4),
        )
        .build(),
        CollisionShape::Capsule {
            radius,
            half_height,
        } => ColliderBuilder::capsule_y(half_height, radius).build(),
    }
}
