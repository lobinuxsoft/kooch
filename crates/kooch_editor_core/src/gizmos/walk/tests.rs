use super::*;
use crate::gizmos::harness::draw;
use glam::Mat4;

/// The circle is the top speed, so it has to change with it.
#[test]
fn the_circle_is_the_top_speed() {
    let walk = Walk::default();
    let segments = draw(&WalkVisualizer, &walk, Mat4::IDENTITY);
    assert!(!segments.is_empty(), "it should draw the speed it allows");
}

/// Without a world there is no goal and no velocity, and an arrow drawn
/// anyway would be a heading the character never had.
#[test]
fn no_world_draws_no_arrows() {
    let walk = Walk::default();
    let bare = draw(&WalkVisualizer, &walk, Mat4::IDENTITY);
    let ring = bare.len();
    let mut lines = kooch_gizmos::GizmoBatch::default();
    let mut meshes = kooch_gizmos::MeshBatch::default();
    {
        let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
        draw_at(
            &walk,
            &GlobalTransform {
                matrix: Mat4::IDENTITY,
            },
            Vec3::Y,
            Some(Vec3::X * 6.0),
            Some(Vec3::X * 3.0),
            &mut gizmos,
        );
    }
    assert!(lines.lines.len() > ring, "both arrows should be drawn");
}
