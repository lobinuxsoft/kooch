use super::*;

#[test]
fn vertex_size_is_32_bytes() {
    assert_eq!(std::mem::size_of::<MeshVertex>(), 32);
}

#[test]
fn aabb_starts_empty_and_expands() {
    let mut aabb = Aabb::empty();
    assert!(aabb.is_empty());
    aabb.expand(Vec3::new(1.0, 2.0, 3.0));
    aabb.expand(Vec3::new(-1.0, 5.0, 0.0));
    assert!(!aabb.is_empty());
    assert_eq!(aabb.min, Vec3::new(-1.0, 2.0, 0.0));
    assert_eq!(aabb.max, Vec3::new(1.0, 5.0, 3.0));
}
