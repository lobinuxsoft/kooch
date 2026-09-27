use super::*;
use crate::gizmos::harness::{arrows, draw, reach};
use glam::{Mat4, Quat};

fn cube() -> BoxGravity {
    BoxGravity {
        half_extents: Vec3::splat(10.0),
        rounding: 0.0,
        range: Vec3::ZERO,
        falloff: 0.0,
        ..Default::default()
    }
}

/// The claim: each face pulls along its own normal. Six arrows, one per
/// face, each pointing at the solid.
#[test]
fn every_face_gets_an_arrow_along_its_own_normal() {
    let shafts = arrows(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY);
    assert_eq!(shafts.len(), 6, "expected one arrow per face");
    for normal in FACES {
        assert!(
            shafts.iter().any(|s| s.abs_diff_eq(-normal, 1e-3)),
            "no arrow pulls along {normal}; got {shafts:?}",
        );
    }
}

#[test]
fn the_solid_is_drawn_at_its_half_extents() {
    let corner = Vec3::splat(10.0).length();
    let reach = reach(&draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY));
    // Plus the arrows, which stand `ARROW` off each face — shorter than
    // the corner diagonal, so the corner still sets the reach.
    assert!(
        (reach - corner).abs() < 0.1,
        "reached {reach}, wanted {corner}",
    );
}

/// `rounding` decides where gravity starts turning, and without the
/// inner box on screen it is a number with nothing to check it against.
#[test]
fn rounding_draws_the_box_it_actually_clamps_against() {
    let hard = draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY);
    let rounded = draw(
        &BoxGravityVisualizer,
        &BoxGravity {
            rounding: 4.0,
            ..cube()
        },
        Mat4::IDENTITY,
    );
    assert!(
        rounded.len() > hard.len(),
        "rounding drew nothing extra: {} then {}",
        hard.len(),
        rounded.len(),
    );
}

/// A planet with a reach has to show it, or "does this pull that
/// platform" is unanswerable without running the game.
#[test]
fn the_reach_is_drawn_when_the_field_is_limited() {
    let limited = BoxGravity {
        range: Vec3::splat(20.0),
        falloff: 5.0,
        ..cube()
    };
    let far = reach(&draw(&BoxGravityVisualizer, &limited, Mat4::IDENTITY));
    let near = reach(&draw(&BoxGravityVisualizer, &cube(), Mat4::IDENTITY));
    assert!(far > near + 20.0, "{near} then {far}");
}

/// 🔴 #1324: one shell showed where gravity ENDS and nothing about where it begins to fade, so
/// `falloff` was a number with nothing on screen to check it against. Two shells, and the band
/// between them is it.
#[test]
fn the_falloff_has_a_shell_of_its_own() {
    let hard = BoxGravity {
        range: Vec3::splat(20.0),
        falloff: 0.0,
        ..cube()
    };
    let faded = BoxGravity {
        falloff: 5.0,
        ..hard
    };
    let boxes = |field: &BoxGravity| draw(&BoxGravityVisualizer, field, Mat4::IDENTITY).len();
    assert!(
        boxes(&faded) > boxes(&hard),
        "the fade drew no shell of its own",
    );
    // And it stands where the fade ends, not where full strength does.
    let far = reach(&draw(&BoxGravityVisualizer, &faded, Mat4::IDENTITY));
    let full = reach(&draw(&BoxGravityVisualizer, &hard, Mat4::IDENTITY));
    assert!(
        (far - full - 5.0 * 3f32.sqrt()).abs() < 0.5,
        "{full} then {far}"
    );
}

/// One axis may reach further than another, and the shell has to show that rather than splitting
/// the difference.
#[test]
fn an_axis_reaches_on_its_own() {
    let tall = BoxGravity {
        range: Vec3::new(5.0, 40.0, 5.0),
        falloff: 0.0,
        ..cube()
    };
    let drawn = draw(&BoxGravityVisualizer, &tall, Mat4::IDENTITY);
    let top = drawn
        .iter()
        .flat_map(|(a, b)| [a.y, b.y])
        .fold(f32::MIN, f32::max);
    let side = drawn
        .iter()
        .flat_map(|(a, b)| [a.x, b.x])
        .fold(f32::MIN, f32::max);
    assert!(
        (top - 50.0).abs() < 0.1,
        "the Y shell reached {top}, wanted 50"
    );
    assert!(
        (side - 15.0).abs() < 0.1,
        "the X shell reached {side}, wanted 15"
    );
}

/// 🔴 #1326: one zero used to erase both shells, so a field that stops looked like one that never
/// does. Only a field with no cutoff at all draws none.
#[test]
fn a_zero_axis_still_draws_its_shells() {
    let flat = BoxGravity {
        range: Vec3::new(30.0, 0.0, 30.0),
        falloff: 0.0,
        ..cube()
    };
    let boxes = |field: &BoxGravity| draw(&BoxGravityVisualizer, field, Mat4::IDENTITY).len();
    assert!(
        boxes(&flat) > boxes(&cube()),
        "a field with one zero axis drew no reach at all",
    );
}

/// Turning the planet turns its faces, so the arrows have to follow —
/// the same round trip through the entity's rotation the solver makes.
#[test]
fn the_faces_turn_with_the_entity() {
    let turned = Mat4::from_quat(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let shafts = arrows(&BoxGravityVisualizer, &cube(), turned);

    // Local +X turned a quarter turn about +Z points along +Y, so the
    // arrow onto that face now pulls along -Y.
    assert!(
        shafts.iter().any(|s| s.abs_diff_eq(Vec3::NEG_Y, 1e-3)),
        "no arrow follows the rotated +X face: {shafts:?}",
    );
}

/// A field's space is rigid, so its extents are metres and the entity's scale places it without
/// resizing it.
#[test]
fn a_scaled_box_is_the_same_size() {
    let field = BoxGravity {
        half_extents: Vec3::splat(5.0),
        rounding: 0.5,
        range: Vec3::splat(20.0),
        falloff: 5.0,
        ..Default::default()
    };
    let plain = reach(&draw(&BoxGravityVisualizer, &field, Mat4::IDENTITY));
    let scaled = reach(&draw(
        &BoxGravityVisualizer,
        &field,
        Mat4::from_scale(Vec3::splat(8.0)),
    ));
    assert!((scaled - plain).abs() < 1e-3, "{plain} then {scaled}");
}
