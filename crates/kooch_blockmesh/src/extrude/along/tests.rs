use glam::Vec3;
use kooch_ecs::spline::Knot;

use crate::BlockMesh;

fn cube() -> BlockMesh {
    BlockMesh::cuboid(Vec3::splat(0.5))
}

/// The face whose outward normal points along `axis`.
fn facing(mesh: &BlockMesh, axis: Vec3) -> u32 {
    (0..mesh.face_count())
        .find(|face| mesh.face_normal(*face).unwrap().dot(axis) > 0.99)
        .expect("a cube faces every axis") as u32
}

/// A straight line of knots along +Z, at the given distances.
fn line(at: &[f32]) -> Vec<Knot> {
    at.iter().map(|z| Knot::at(Vec3::Z * *z)).collect()
}

/// A run down a straight curve is the straight extrude, or the curve is not carrying the face —
/// it is re-deriving it.
#[test]
fn a_straight_run_matches_extrude() {
    let (mut along, mut straight) = (cube(), cube());
    let face = facing(&along, Vec3::Z);
    along
        .extrude_along(&[face], &line(&[0.0, 2.0]), false, 1, false)
        .expect("a straight curve extrudes");
    straight
        .extrude(&[face], Vec3::Z * 2.0)
        .expect("the straight extrude is the reference");

    assert_eq!(along.positions().len(), straight.positions().len());
    for (ran, pulled) in along.positions().iter().zip(straight.positions()) {
        assert!(
            ran.distance(*pulled) < 1.0e-4,
            "ran {ran:?} against pulled {pulled:?}",
        );
    }
}

/// 🔴 The knots are 1 m apart and then 9 m apart: even steps of `t` would put three rings in the
/// first metre, which is what makes an extrude visibly bunch up wherever the curve is tight.
#[test]
fn rings_are_evenly_spaced() {
    let mut mesh = cube();
    let face = facing(&mesh, Vec3::Z);
    let extruded = mesh
        .extrude_along(&[face], &line(&[0.0, 1.0, 10.0]), false, 4, false)
        .expect("the curve has length");

    // Where the last ring landed, read back through the walls it stitched: each wall carries one
    // corner per ring, so the rings' z values are the walls' z values.
    let mut depths: Vec<f32> = extruded
        .faces
        .iter()
        .chain(&extruded.walls)
        .flat_map(|face| mesh.face(*face as usize).unwrap())
        .map(|corner| mesh.positions()[*corner as usize].z)
        .collect();
    depths.sort_by(f32::total_cmp);
    depths.dedup_by(|a, b| (*a - *b).abs() < 1.0e-3);

    // The face starts at 0.5 and four rings walk the 10 m curve, so every step is 2.5 m.
    //
    // 🔴 To within 6%, not to the float: `arc::walked` is 16 chords per segment, and the second
    // segment here is 9 m long. Even steps of `t` would put the first ring at 1 m, not 3.
    assert_eq!(depths.len(), 5, "rings at {depths:?}");
    for step in depths.windows(2) {
        let gap = step[1] - step[0];
        assert!((gap - 2.5).abs() < 0.15, "rings at {depths:?}");
    }
}
