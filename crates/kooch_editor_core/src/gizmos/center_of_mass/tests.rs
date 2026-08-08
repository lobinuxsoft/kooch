use super::*;
use glam::{Mat4, Quat};
use kooch_gizmos::{GizmoBatch, MeshBatch};

fn draw(body: &PhysicsBody, matrix: Mat4) -> Vec<(Vec3, Vec3)> {
    let mut lines = GizmoBatch::default();
    let mut meshes = MeshBatch::default();
    let mut gizmos = Gizmos::new(&mut lines, &mut meshes);
    CenterOfMassVisualizer.draw(body, &GlobalTransform { matrix }, &mut gizmos);
    lines.lines.iter().map(|s| (s.start, s.end)).collect()
}

fn enabled(center: Vec3) -> PhysicsBody {
    PhysicsBody {
        center_of_mass_enabled: true,
        center_of_mass: center,
        ..Default::default()
    }
}

/// With the override off the solver derives the point from the shapes,
/// and a marker drawn at a guess would be worse than none.
#[test]
fn nothing_is_drawn_without_an_authored_centre() {
    assert!(draw(&PhysicsBody::default(), Mat4::IDENTITY).is_empty());
}

#[test]
fn an_authored_centre_is_drawn_where_it_was_authored() {
    let center = Vec3::new(0.0, -0.4, 0.0);
    let segments = draw(&enabled(center), Mat4::IDENTITY);

    assert!(!segments.is_empty(), "nothing was drawn");
    // The crosshair arms meet at the point, so it is the midpoint of
    // the three longest segments.
    assert!(
        segments
            .iter()
            .any(|(a, b)| ((*a + *b) / 2.0).abs_diff_eq(center, 1e-4)),
        "no segment is centred on {center}",
    );
}

/// The offset is in the entity's local space, so it has to move with
/// the entity — a marker anchored at the world origin would be a lie
/// the moment anything is dragged.
#[test]
fn the_marker_follows_the_entity() {
    let center = Vec3::new(0.0, -0.4, 0.0);
    let moved = Mat4::from_translation(Vec3::new(5.0, 0.0, 0.0));
    let segments = draw(&enabled(center), moved);

    assert!(
        segments
            .iter()
            .any(|(a, b)| ((*a + *b) / 2.0).abs_diff_eq(center + Vec3::X * 5.0, 1e-4)),
        "the marker did not move with the entity",
    );
}

/// And it scales with it, the same way the solver scales the offset
/// when it builds the body.
#[test]
fn the_offset_scales_with_the_entity() {
    let center = Vec3::new(0.0, -0.5, 0.0);
    let scaled = Mat4::from_scale(Vec3::splat(4.0));
    let segments = draw(&enabled(center), scaled);

    assert!(
        segments
            .iter()
            .any(|(a, b)| ((*a + *b) / 2.0).abs_diff_eq(Vec3::new(0.0, -2.0, 0.0), 1e-4)),
        "a 4x scaled entity should put a -0.5 offset at -2.0",
    );
}

/// A rotated entity carries its offset around with it.
#[test]
fn the_offset_rotates_with_the_entity() {
    let center = Vec3::new(1.0, 0.0, 0.0);
    let turned = Mat4::from_quat(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2));
    let segments = draw(&enabled(center), turned);

    // +X rotated a quarter turn about Y lands on -Z.
    assert!(
        segments
            .iter()
            .any(|(a, b)| ((*a + *b) / 2.0).abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-3)),
        "the offset did not rotate with the entity",
    );
}
