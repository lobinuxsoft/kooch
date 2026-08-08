use super::*;

/// A hard cube, big enough that the test distances are unambiguous.
fn cube() -> BoxGravity {
    BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 0.0,
        range: 0.0,
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

/// Inside the solid there is no surface to fall towards. A body there
/// is inside the rock, and inventing a direction for it would be a
/// force that shoots it out of the planet.
#[test]
fn inside_the_solid_nothing_pulls() {
    let field = cube();
    assert_eq!(field.acceleration_at_local(Vec3::ZERO), Vec3::ZERO);
    assert_eq!(field.acceleration_at_local(Vec3::splat(9.0)), Vec3::ZERO);
}

/// Rounding equal to the half-extents shrinks the box to its centre,
/// and the closest-point field around a point *is* a sphere. The dial
/// runs all the way from cube to planet.
#[test]
fn full_rounding_makes_a_sphere() {
    let field = BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 10.0,
        range: 0.0,
        ..Default::default()
    };
    // Over the corner diagonal, a cube would still pull along the
    // diagonal — but so does a sphere, so probe somewhere they differ:
    // over a face, a cube pulls straight down and a sphere pulls at the
    // centre, which from here is the same. Use an oblique point.
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
        range: 5.0,
        falloff: 10.0,
        ..Default::default()
    };
    // Distances are measured from the surface, not from the centre.
    assert_eq!(field.influence(4.0), 1.0);
    assert!((field.influence(10.0) - 0.5).abs() < 1e-4);
    assert_eq!(field.influence(16.0), 0.0);
    assert_eq!(
        field.acceleration_at_local(Vec3::new(0.0, 30.0, 0.0)),
        Vec3::ZERO,
    );
}

/// Zero range is unlimited, or a planet would need its reach retyped
/// every time it grew.
#[test]
fn an_unlimited_field_never_fades() {
    assert_eq!(cube().influence(10_000.0), 1.0);
}
