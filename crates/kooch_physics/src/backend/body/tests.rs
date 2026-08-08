use super::*;

#[test]
fn body_desc_dynamic_default_at_origin() {
    let desc = BodyDesc::dynamic(CollisionShape::Sphere { radius: 1.0 }, 5.0);
    assert_eq!(desc.kind, BodyKind::Dynamic);
    assert_eq!(desc.position, Vec3::ZERO);
    assert_eq!(desc.rotation, Quat::IDENTITY);
    assert_eq!(desc.mass, 5.0);
}

#[test]
fn body_desc_static_uses_position() {
    let pos = Vec3::new(1.0, 2.0, 3.0);
    let desc = BodyDesc::static_at(
        CollisionShape::Cuboid {
            half_extents: Vec3::splat(0.5),
        },
        pos,
    );
    assert_eq!(desc.kind, BodyKind::Static);
    assert_eq!(desc.position, pos);
    assert_eq!(desc.mass, 0.0);
}

#[test]
fn collision_shapes_cover_three_primitives() {
    let _s = CollisionShape::Sphere { radius: 1.0 };
    let _c = CollisionShape::Cuboid {
        half_extents: Vec3::ONE,
    };
    let _cap = CollisionShape::Capsule {
        radius: 0.5,
        half_height: 1.0,
    };
}
