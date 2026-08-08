use super::*;

#[test]
fn ring_closes_the_seam_with_a_duplicate() {
    let r = ring(1.0, 0.0, 4);
    assert_eq!(r.len(), 5, "the seam vertex is not duplicated");
    assert!(
        r[0].abs_diff_eq(r[4], 1e-5),
        "first and last ring positions differ: {:?} vs {:?}",
        r[0],
        r[4]
    );
}

#[test]
fn ring_sits_on_the_circle_at_the_requested_height() {
    for p in ring(2.0, 3.0, 8) {
        assert!((Vec2::new(p.x, p.z).length() - 2.0).abs() < 1e-5);
        assert_eq!(p.y, 3.0);
    }
}

#[test]
fn builder_normalises_normals() {
    let mut b = MeshBuilder::default();
    b.vertex(Vec3::ZERO, Vec3::new(0.0, 5.0, 0.0), Vec2::ZERO);
    let mesh = b.build();
    assert_eq!(mesh.vertices[0].normal, [0.0, 1.0, 0.0]);
}
