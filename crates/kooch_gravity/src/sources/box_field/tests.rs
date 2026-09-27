use super::*;

/// A hard cube, big enough that the test distances are unambiguous.
fn cube() -> BoxGravity {
    BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 0.0,
        range: Vec3::ZERO,
        falloff: 0.0,
        ..Default::default()
    }
}

/// The claim the component makes: over a face, gravity is that face's
/// normal — and the same one everywhere on it, or you could not walk
/// across it without leaning.
#[test]
fn each_face_pulls_along_its_own_normal() {
    let field = cube();
    for (probe, wanted) in [
        (Vec3::new(0.0, 15.0, 0.0), Vec3::NEG_Y),
        (Vec3::new(0.0, -15.0, 0.0), Vec3::Y),
        (Vec3::new(15.0, 0.0, 0.0), Vec3::NEG_X),
        (Vec3::new(-15.0, 0.0, 0.0), Vec3::X),
        (Vec3::new(0.0, 0.0, 15.0), Vec3::NEG_Z),
        (Vec3::new(0.0, 0.0, -15.0), Vec3::Z),
        // Off-centre on the +Y face, still straight down.
        (Vec3::new(9.0, 15.0, -9.0), Vec3::NEG_Y),
    ] {
        let accel = field.acceleration_at_local(probe);
        assert!(
            accel.normalize().abs_diff_eq(wanted, 1e-4),
            "at {probe} the pull was {accel}, wanted {wanted}",
        );
    }
}

/// The reason no edge case is written anywhere: the closest-point
/// direction is continuous, so walking over an edge turns gravity
/// smoothly instead of flipping it in one step.
#[test]
fn gravity_turns_continuously_around_an_edge() {
    let field = cube();
    // A quarter arc around the +X/+Y edge, from clearly over the top
    // face to clearly out from the side one, at a constant 5 m.
    const EDGE: Vec3 = Vec3::new(10.0, 10.0, 0.0);
    const STEPS: u32 = 60;
    let sweep = std::f32::consts::FRAC_PI_2 + 1.2;

    let mut previous: Option<Vec3> = None;
    let mut first = Vec3::ZERO;
    for step in 0..=STEPS {
        let angle = -0.6 + sweep * step as f32 / STEPS as f32;
        let probe = EDGE + Vec3::new(angle.sin(), angle.cos(), 0.0) * 5.0;
        let now = field.acceleration_at_local(probe).normalize();
        match previous {
            None => first = now,
            Some(before) => {
                let turn = before.dot(now).clamp(-1.0, 1.0).acos().to_degrees();
                assert!(turn < 10.0, "gravity jumped {turn}° in one step at {probe}");
            }
        }
        previous = Some(now);
    }

    // And it did turn the whole quarter: a field that never moved at
    // all would pass the check above trivially.
    assert!(first.abs_diff_eq(Vec3::NEG_Y, 1e-3), "started at {first}");
    assert!(
        previous.expect("sampled").abs_diff_eq(Vec3::NEG_X, 1e-3),
        "ended at {:?}",
        previous,
    );
}

/// Diagonally out from a corner, all three faces are equally near.
#[test]
fn a_corner_pulls_along_its_diagonal() {
    let field = cube();
    let accel = field.acceleration_at_local(Vec3::splat(20.0));
    assert!(
        accel
            .normalize()
            .abs_diff_eq(Vec3::splat(-1.0).normalize(), 1e-4),
        "{accel}",
    );
}

/// 🔴 #1324: inside the solid the gradient vanishes, and the old answer was nothing at all — a
/// body that clipped in or spawned there floated, while the planet still claimed it. It keeps
/// falling towards the face it is under, which is the direction it had a step before crossing.
#[test]
fn inside_the_solid_keeps_its_face() {
    let field = cube();
    // Just under the +Y face: still falling the way it was, which is down into the solid.
    let under = field.acceleration_at_local(Vec3::new(0.0, 9.0, 0.0));
    assert!(under.normalize().abs_diff_eq(Vec3::NEG_Y, 1e-4), "{under}");
    // And under −X, the nearest face there: inwards again, so towards +X.
    let side = field.acceleration_at_local(Vec3::new(-9.0, 1.0, 0.5));
    assert!(side.normalize().abs_diff_eq(Vec3::X, 1e-4), "{side}");
    // The exact centre is the one place with no face to name.
    assert_eq!(field.acceleration_at_local(Vec3::ZERO), Vec3::ZERO);
}

/// Crossing a face must not jump: just outside, gravity is that face's inward normal, and just
/// inside it is the same vector.
#[test]
fn the_surface_is_continuous() {
    let field = cube();
    let outside = field.acceleration_at_local(Vec3::new(0.0, 10.01, 0.0));
    let inside = field.acceleration_at_local(Vec3::new(0.0, 9.99, 0.0));
    assert!(
        outside.normalize().abs_diff_eq(inside.normalize(), 1e-3),
        "{outside} outside against {inside} inside",
    );
}

/// One axis may reach further than another, and each covers both of its faces.
#[test]
fn an_axis_reaches_on_its_own() {
    let field = BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 0.0,
        range: Vec3::new(30.0, 5.0, 30.0),
        falloff: 0.0,
        ..Default::default()
    };
    // 8 m past +Y: beyond its 5 m reach, and a hard edge.
    assert_eq!(
        field.acceleration_at_local(Vec3::new(0.0, 18.0, 0.0)),
        Vec3::ZERO,
    );
    // The same 8 m past −Y: the axis covers both faces, so it stops there too.
    assert_eq!(
        field.acceleration_at_local(Vec3::new(0.0, -18.0, 0.0)),
        Vec3::ZERO,
    );
    // And 8 m past +X, which reaches 30.
    let side = field.acceleration_at_local(Vec3::new(18.0, 0.0, 0.0));
    assert!(side.length() > 0.0, "the long axis stopped pulling too");
}

/// 🔴 #1326: a zero on one axis is a reach of nothing there, not "unlimited" — reading it as
/// unlimited made a field with one zero pull for ever on that axis while the gizmo drew nothing at
/// all. Only all-zero is the planet with no cutoff.
#[test]
fn one_zero_axis_is_not_unlimited() {
    let field = BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 0.0,
        range: Vec3::new(30.0, 0.0, 30.0),
        falloff: 0.0,
        ..Default::default()
    };
    assert!(
        !field.is_unlimited(),
        "one zero made the whole field endless"
    );
    // A metre past +Y, where the reach is nothing.
    assert_eq!(
        field.acceleration_at_local(Vec3::new(0.0, 11.0, 0.0)),
        Vec3::ZERO,
    );
    // The other axes are untouched.
    let side = field.acceleration_at_local(Vec3::new(18.0, 0.0, 0.0));
    assert!(side.length() > 0.0, "a zero on Y silenced X");

    let endless = BoxGravity {
        range: Vec3::ZERO,
        ..field
    };
    assert!(
        endless.is_unlimited(),
        "all zero is the field with no cutoff"
    );
}

/// A negative reach is the same reach: the component is a distance, and a minus sign in the
/// Inspector must not turn the field off.
#[test]
fn a_negative_reach_is_its_size() {
    let field = BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 0.0,
        range: Vec3::splat(-30.0),
        falloff: 0.0,
        ..Default::default()
    };
    let side = field.acceleration_at_local(Vec3::new(18.0, 0.0, 0.0));
    assert!(side.length() > 0.0, "a negative reach read as no reach");
}

/// Rounding equal to the half-extents shrinks the box to its centre,
/// and the closest-point field around a point *is* a sphere. The dial
/// runs all the way from cube to planet.
#[test]
fn full_rounding_makes_a_sphere() {
    let field = BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 10.0,
        range: Vec3::ZERO,
        ..Default::default()
    };
    // Cube and sphere agree over the diagonal and over a face centre, so probe an oblique point
    // where they differ.
    let probe = Vec3::new(4.0, 20.0, 0.0);
    let accel = field.acceleration_at_local(probe);
    assert!(
        accel.normalize().abs_diff_eq(-probe.normalize(), 1e-4),
        "a fully rounded box should pull at its centre: {accel}",
    );
}

/// And with no rounding the same probe pulls straight down instead,
/// which is what makes the previous test mean something.
#[test]
fn a_hard_cube_does_not_pull_at_its_centre() {
    let accel = cube().acceleration_at_local(Vec3::new(4.0, 20.0, 0.0));
    assert!(accel.normalize().abs_diff_eq(Vec3::NEG_Y, 1e-4), "{accel}");
}

#[test]
fn the_field_fades_past_its_range() {
    let field = BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 0.0,
        range: Vec3::splat(5.0),
        falloff: 10.0,
        ..Default::default()
    };
    // Distances are measured from the surface, not from the centre.
    let above = |out: f32| Vec3::new(0.0, 10.0 + out, 0.0);
    assert_eq!(field.influence_at_local(above(4.0), 4.0), 1.0);
    assert!((field.influence_at_local(above(10.0), 10.0) - 0.5).abs() < 1e-4);
    assert_eq!(field.influence_at_local(above(16.0), 16.0), 0.0);
    assert_eq!(
        field.acceleration_at_local(Vec3::new(0.0, 30.0, 0.0)),
        Vec3::ZERO,
    );
}

/// Zero range is unlimited, or a planet would need its reach retyped
/// every time it grew.
#[test]
fn an_unlimited_field_never_fades() {
    assert_eq!(
        cube().influence_at_local(Vec3::new(0.0, 10_010.0, 0.0), 10_000.0),
        1.0
    );
}
