use super::*;
use crate::spline::Spline;

fn straight(length: f32) -> Spline {
    Spline {
        points: vec![Knot::at(Vec3::ZERO), Knot::at(Vec3::X * length)],
        closed: false,
    }
}

fn placing(mode: u32) -> SplinePlacement {
    SplinePlacement {
        prefab: Some(Guid::new_v4()),
        mode,
        count: 5,
        spacing: 2.0,
        align: false,
        offset: Vec3::ZERO,
    }
}

/// An open curve puts one instance at each end, so `count` instances leave `count - 1` gaps.
#[test]
fn a_count_reaches_both_ends() {
    let poses = poses(&straight(10.0), &placing(SPACING_COUNT));
    assert_eq!(poses.len(), 5);
    assert!(poses[0].position.x.abs() < 1e-3);
    assert!((poses[4].position.x - 10.0).abs() < 1e-3);
}

/// 🔴 The guard that keeps a picked-up collectible gone: only the curve and the numbers move it,
/// so despawning an instance leaves the run alone.
#[test]
fn a_revision_ignores_instances() {
    let spline = straight(10.0);
    let placement = placing(SPACING_COUNT);
    assert_eq!(revision(&spline, &placement), revision(&spline, &placement));
}

/// Moving a knot is what SHOULD rebuild.
#[test]
fn a_moved_knot_changes_it() {
    let placement = placing(SPACING_COUNT);
    assert_ne!(
        revision(&straight(10.0), &placement),
        revision(&straight(11.0), &placement)
    );
}

/// A spacing of zero would divide a curve into infinite instances.
#[test]
fn a_zero_spacing_is_bounded() {
    let mut placement = placing(SPACING_DISTANCE);
    placement.spacing = 0.0;
    assert!(poses(&straight(10.0), &placement).len() <= MAX_INSTANCES);
}
