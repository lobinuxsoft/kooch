use glam::{Quat, Vec3};
use rapier3d::na::{self, Translation3, Unit, UnitQuaternion, Vector3};
use rapier3d::prelude::*;

use crate::backend::CollisionShape;

pub(super) fn vec3_to_na(v: Vec3) -> Vector3<f32> {
    Vector3::new(v.x, v.y, v.z)
}

pub(super) fn na_to_vec3(v: Vector3<f32>) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

pub(super) fn point(v: Vec3) -> na::Point3<f32> {
    na::Point3::new(v.x, v.y, v.z)
}

pub(super) fn isometry(pos: Vec3, rot: Quat) -> Isometry<Real> {
    let translation = Translation3::new(pos.x, pos.y, pos.z);
    let rotation = UnitQuaternion::from_quaternion(na::Quaternion::new(
        rot.w, rot.x, rot.y, rot.z,
    ));
    Isometry::from_parts(translation, rotation)
}

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

#[allow(unused)]
pub(super) fn _unit_y_helper() -> Unit<Vector3<f32>> {
    Vector3::y_axis()
}
