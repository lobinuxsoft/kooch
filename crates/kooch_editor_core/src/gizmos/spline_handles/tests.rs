use super::*;

fn curve() -> Spline {
    Spline {
        points: vec![
            Knot::at(Vec3::ZERO),
            Knot::at(Vec3::new(1.0, 0.0, 0.0)),
            Knot::at(Vec3::new(2.0, 0.0, 0.0)),
        ],
        closed: false,
    }
}

/// 🔴 Dragging an `Auto` handle promotes the knot, or the handle springs back to where the
/// neighbours put it and the author concludes the editor is broken.
#[test]
fn a_dragged_auto_handle_promotes() {
    let knot = Knot::at(Vec3::ZERO);
    assert_eq!(knot.mode, TANGENT_AUTO);

    let after = moved(knot, Part::Leaving, Vec3::new(0.0, 1.0, 0.0));
    assert_eq!(after.mode, TANGENT_ALIGNED);
    assert!(after.leaving.length() > 0.0);
}

/// Aligned is one line through the knot, so moving either end moves the other. Breaking the pair is
/// a mode chosen in the Inspector, never a side effect of a drag.
#[test]
fn aligned_moves_the_pair() {
    let knot = Knot {
        mode: TANGENT_ALIGNED,
        leaving: Vec3::new(1.0, 0.0, 0.0),
        ..Knot::at(Vec3::ZERO)
    };
    let after = moved(knot, Part::Arriving, Vec3::new(0.0, 1.0, 0.0));

    assert_eq!(after.mode, TANGENT_ALIGNED);
    // Arriving points back, so pulling it up sends `leaving` down.
    assert!(after.leaving.y < 0.0, "{}", after.leaving);
}

/// Broken is the only mode where the two are independent.
#[test]
fn broken_moves_one_side() {
    let knot = Knot {
        mode: TANGENT_BROKEN,
        leaving: Vec3::new(1.0, 0.0, 0.0),
        arriving: Vec3::new(-1.0, 0.0, 0.0),
        ..Knot::at(Vec3::ZERO)
    };
    let after = moved(knot, Part::Arriving, Vec3::new(0.0, 1.0, 0.0));

    assert_eq!(after.leaving, knot.leaving, "leaving should not have moved");
    assert!(after.arriving.y > 0.0, "{}", after.arriving);
}

/// A handle dropped on its own knot has no direction left, and a zero tangent flattens the segment
/// to a straight line the author did not ask for.
#[test]
fn a_collapsed_handle_is_ignored() {
    let knot = Knot {
        mode: TANGENT_ALIGNED,
        leaving: Vec3::new(1.0, 0.0, 0.0),
        ..Knot::at(Vec3::ZERO)
    };
    assert_eq!(moved(knot, Part::Leaving, knot.position), knot);
}

/// An open curve's ends have one neighbour, so they get one handle and not two — a grip for a
/// direction the curve takes nowhere is a grip that edits nothing.
#[test]
fn an_open_end_has_one_handle() {
    let spline = curve();
    let found = grips(&spline);
    let parts = |index: usize| {
        found
            .iter()
            .filter(|(grip, _)| grip.index == index)
            .map(|(grip, _)| grip.part)
            .collect::<Vec<_>>()
    };

    assert_eq!(parts(0), vec![Part::Knot, Part::Leaving]);
    assert_eq!(parts(2), vec![Part::Knot, Part::Arriving]);
    assert_eq!(parts(1), vec![Part::Knot, Part::Leaving, Part::Arriving]);
}

/// Closed has no end, so every knot carries both.
#[test]
fn a_closed_curve_has_every_handle() {
    let spline = Spline {
        closed: true,
        ..curve()
    };
    assert_eq!(grips(&spline).len(), spline.points.len() * 3);
}

/// 🔴 `!(step > 0.0)`: a NaN step passes a `<=` test and divides the position into NaN, which puts
/// the knot nowhere.
#[test]
fn an_unusable_snap_is_ignored() {
    let at = Vec3::new(0.3, 1.7, -2.2);
    for step in [0.0, -1.0, f32::NAN] {
        assert_eq!(snapped(at, step), at, "{step}");
    }
    assert_eq!(snapped(at, 1.0), Vec3::new(0.0, 2.0, -2.0));
}

/// A grip sits exactly where the gizmo draws the handle's tip, or it is grabbable a pixel off what
/// the author is looking at.
#[test]
fn a_handle_grip_sits_on_its_tip() {
    let spline = curve();
    let (grip, local) = grips(&spline)
        .into_iter()
        .find(|(grip, _)| grip.index == 1 && grip.part == Part::Leaving)
        .expect("the middle knot leaves");

    let knot = spline.points[grip.index];
    let leaving = eval::tangents(&spline.points, spline.closed, 1, 2).0;
    assert!(local.distance(knot.position + leaving * HANDLE_SCALE) < 1e-6);
}
