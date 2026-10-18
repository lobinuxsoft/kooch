use glam::Vec3;
use kooch_ecs::spline::{Knot, Spline};

use super::{PROFILE_ROUND, PROFILE_SQUARE, SplineSweep, sweep};
use crate::Adjacency;

/// An S, so the curve has an inflection — the place a Frenet frame would flip and take the whole
/// ring with it.
fn curve() -> Spline {
    Spline {
        points: vec![
            Knot::at(Vec3::new(-3.0, 0.0, 0.0)),
            Knot::at(Vec3::new(-1.0, 0.0, 2.0)),
            Knot::at(Vec3::new(1.0, 0.0, -2.0)),
            Knot::at(Vec3::new(3.0, 0.0, 0.0)),
        ],
        closed: false,
    }
}

/// 🔴 A hole makes every later extrude wrong, and the sweep is the first shape whose geometry the
/// author can bend after it is built.
#[test]
fn a_sweep_is_closed() {
    let curve = curve();
    for profile in [PROFILE_ROUND, PROFILE_SQUARE] {
        for closed in [false, true] {
            let mesh = sweep(
                &curve.points,
                closed,
                &SplineSweep {
                    profile,
                    ..SplineSweep::default()
                },
            );
            assert!(
                Adjacency::of(&mesh).is_closed(),
                "profile {profile} closed {closed} leaks"
            );
        }
    }
}

/// A tip welds its ring into one point, and a quad with a doubled corner is a face with no area.
#[test]
fn a_tapered_end_closes() {
    let mut curve = curve();
    curve.points.last_mut().expect("four knots").radius = 0.0;
    let mesh = sweep(&curve.points, false, &SplineSweep::default());
    assert!(Adjacency::of(&mesh).is_closed(), "the tip leaks");
}

/// Fewer than two knots is a point, and a mesh built from one is a shape with no inside.
#[test]
fn a_short_curve_sweeps_nothing() {
    let one = vec![Knot::at(Vec3::ZERO)];
    assert_eq!(sweep(&one, false, &SplineSweep::default()).face_count(), 0);
    assert_eq!(sweep(&[], false, &SplineSweep::default()).face_count(), 0);
}

/// 🔴 The bore doubles every surface and its own normals point the other way. A hole here is a
/// tunnel the character falls out of, and no screenshot shows it.
#[test]
fn a_walled_sweep_is_closed() {
    let curve = curve();
    for profile in [PROFILE_ROUND, PROFILE_SQUARE] {
        for closed in [false, true] {
            for caps in [false, true] {
                let mesh = sweep(
                    &curve.points,
                    closed,
                    &SplineSweep {
                        profile,
                        caps,
                        wall: 0.2,
                        ..SplineSweep::default()
                    },
                );
                assert!(
                    Adjacency::of(&mesh).is_closed(),
                    "profile {profile} closed {closed} caps {caps} leaks"
                );
            }
        }
    }
}

/// A wall thicker than the radius pinches the bore shut, and the quads there go to triangles.
#[test]
fn a_pinched_bore_closes() {
    let mut curve = curve();
    curve.points.last_mut().expect("four knots").radius = 0.0;
    let mesh = sweep(
        &curve.points,
        false,
        &SplineSweep {
            wall: 0.4,
            ..SplineSweep::default()
        },
    );
    assert!(Adjacency::of(&mesh).is_closed(), "the pinch leaks");
}
