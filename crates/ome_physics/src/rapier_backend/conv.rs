use rapier3d::prelude::*;

use glam::Vec3;

use crate::backend::CollisionShape;

/// Builds the Rapier collider for an engine shape.
///
/// Since Rapier 0.32 the math types *are* glam's, so this module no
/// longer translates vectors or quaternions — only shapes, which have no
/// engine-side equivalent.
/// `offset` becomes the collider's position relative to its parent body,
/// which is how rapier expresses a shape that is not centred on the body.
pub(super) fn collider_for(shape: CollisionShape, offset: Vec3) -> Collider {
    builder_for(shape).translation(offset).build()
}

/// The un-built builder, so the offset is applied in one place.
fn builder_for(shape: CollisionShape) -> ColliderBuilder {
    match shape {
        CollisionShape::Sphere { radius } => ColliderBuilder::ball(radius),
        CollisionShape::Cuboid { half_extents } => ColliderBuilder::cuboid(
            half_extents.x.max(1e-4),
            half_extents.y.max(1e-4),
            half_extents.z.max(1e-4),
        ),
        CollisionShape::Capsule {
            radius,
            half_height,
        } => ColliderBuilder::capsule_y(half_height, radius),
    }
}
