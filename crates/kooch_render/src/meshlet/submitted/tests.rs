use super::*;

fn key(part: u32) -> DrawKey {
    (Entity::new(1, 0), part)
}

#[test]
fn a_late_reader_reads_everything() {
    let mut rows = Submitted::default();
    rows.begin();
    rows.set(key(1), Mat4::IDENTITY, MeshRenderer::default());
    let seen = rows.changes();
    rows.begin();
    rows.set(key(2), Mat4::IDENTITY, MeshRenderer::default());
    let changed: Vec<DrawKey> = rows.since(seen).map(|k| k.collect()).unwrap_or_default();
    assert_eq!(changed, [key(2)]);
    rows.begin();
    rows.begin();
    assert!(rows.since(seen).is_none(), "two frames behind reads all");
}

#[test]
fn a_removed_row_keeps_others_found() {
    let mut rows = Submitted::default();
    rows.set(key(1), Mat4::IDENTITY, MeshRenderer::default());
    rows.set(
        key(2),
        Mat4::from_scale(glam::Vec3::splat(2.0)),
        MeshRenderer::default(),
    );
    rows.remove(key(1));
    assert!(rows.get(key(1)).is_none());
    assert_eq!(rows.get(key(2)).map(|(m, _)| m.x_axis.x), Some(2.0));
}
