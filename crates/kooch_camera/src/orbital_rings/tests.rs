use super::*;

/// The three rings, as Cinemachine ships them.
fn rings() -> OrbitalRings {
    OrbitalRings::default()
}

/// Looking down −Z from the target, so a point reads as `(height, radius back)`.
const BACK: Vec3 = Vec3::Z;
const UP: Vec3 = Vec3::Y;

/// The ends of the range land **on** the rings they name, whatever the curvature does between them:
/// the phantom knots shape the ends and are never reached.
#[test]
fn the_ends_are_the_rings() {
    let r = rings();
    let bottom = r.at(0.0, BACK, UP);
    let top = r.at(1.0, BACK, UP);
    assert!((bottom.y - r.bottom_height).abs() < 1e-4, "{bottom:?}");
    assert!((bottom.z - r.bottom_radius).abs() < 1e-4, "{bottom:?}");
    assert!((top.y - r.top_height).abs() < 1e-4, "{top:?}");
    assert!((top.z - r.top_radius).abs() < 1e-4, "{top:?}");
}

/// And the middle is the centre ring, which is where the two Bézier segments meet.
#[test]
fn the_middle_is_the_centre_ring() {
    let r = rings();
    let at = r.at(0.5, BACK, UP);
    assert!((at.y - r.center_height).abs() < 1e-4, "{at:?}");
    assert!((at.z - r.center_radius).abs() < 1e-4, "{at:?}");
}

/// 🔴 The whole feature: the radius **changes** along the surface. A sphere holds it, and holding it
/// is what the rings exist not to do.
#[test]
fn the_radius_moves_along_the_surface() {
    let r = rings();
    let radius = |t: f32| r.at(t, BACK, UP).z;
    // The centre ring is the widest of the three, so the walk out and back in is visible at all.
    assert!(radius(0.5) > radius(0.0) + 1.0);
    assert!(radius(0.5) > radius(1.0) + 1.0);
}

/// The height only ever climbs: a surface that doubled back would put two pitches at one place.
#[test]
fn the_height_only_climbs() {
    let r = rings();
    let mut last = f32::NEG_INFINITY;
    for step in 0..=40 {
        let at = r.at(step as f32 / 40.0, BACK, UP).y;
        assert!(at > last - 1e-3, "it dipped at {step}: {at} after {last}");
        last = at;
    }
}

/// 🔴 The circles lie on the plane `up` is normal to and the height runs along it, so on the side of
/// a planet the rig stands on the local horizon. A surface built on world +Y would lie on its side.
#[test]
fn the_surface_stands_on_the_local_up() {
    let r = rings();
    // Up is +X: the horizon is the YZ plane.
    let at = r.at(0.25, Vec3::Z, Vec3::X);
    assert!((at.x - r.at(0.25, BACK, UP).y).abs() < 1e-4, "{at:?}");
    assert!(at.y.abs() < 1e-4, "it left the local horizon: {at:?}");
}

/// Curvature shapes the ends without moving them: the rings are knots the curve passes through.
#[test]
fn curvature_leaves_the_rings_alone() {
    let taut = OrbitalRings {
        spline_curvature: 0.0,
        ..rings()
    };
    let loose = OrbitalRings {
        spline_curvature: 1.0,
        ..rings()
    };
    for t in [0.0, 0.5, 1.0] {
        assert!(
            (taut.at(t, BACK, UP) - loose.at(t, BACK, UP)).length() < 1e-4,
            "the curvature moved a ring at {t}",
        );
    }
    // And between them it does change the shape, or the field would be inert.
    assert!((taut.at(0.25, BACK, UP) - loose.at(0.25, BACK, UP)).length() > 1e-3);
}

/// Past the ends it clamps rather than running off the phantom knots, which are shape and not
/// surface.
#[test]
fn it_clamps_past_the_ends() {
    let r = rings();
    assert_eq!(r.at(-1.0, BACK, UP), r.at(0.0, BACK, UP));
    assert_eq!(r.at(2.0, BACK, UP), r.at(1.0, BACK, UP));
}
