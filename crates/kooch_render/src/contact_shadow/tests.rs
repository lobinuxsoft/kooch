use super::*;
use glam::Vec4;

/// The march recovers metres as `near / ndc.z`, and that identity is
/// the entire reason the camera lost its far plane. Checked against
/// the projection the camera actually builds, not against the
/// algebra that motivated it.
#[test]
fn near_over_ndc_z_is_the_distance_under_the_engines_projection() {
    let near = 0.1;
    let proj = crate::projection::perspective_infinite_rh_reverse_z(
        60.0_f32.to_radians(),
        16.0 / 9.0,
        near,
    );
    for distance in [0.1_f32, 0.5, 2.0, 37.0, 500.0, 100_000.0] {
        let clip = proj * glam::Vec4::new(0.0, 0.0, -distance, 1.0);
        let recovered = near / (clip.z / clip.w);
        assert!(
            (recovered - distance).abs() < distance * 1e-3,
            "at {distance} m the march would think it was at {recovered} m",
        );
    }
}

#[test]
fn the_uniform_matches_the_shader_struct() {
    // ContactShadowView: mat4x4 (64) + near/length/thickness
    // (64..76) + linear_steps (76..80) + frame (80..84) + three
    // scalar pad words (84..96). Scalars rather than a vec2 on
    // purpose — see the shader.
    assert_eq!(std::mem::size_of::<ContactShadowUbo>(), 96);
}

#[test]
fn substitution_leaves_no_placeholder_behind() {
    let src = contact_shadow_shader(3, 4);
    assert!(
        !src.contains("{{"),
        "a surviving placeholder fails to parse"
    );
    assert!(src.contains("@group(0) @binding(3)"));
    assert!(src.contains("@group(0) @binding(4)"));
}

/// An unset variable has to be distinguishable from `0`: one leaves the
/// author's value standing, the other turns the march off everywhere.
#[test]
fn unset_is_not_zero() {
    assert_eq!(parse_steps(None), None);
    assert_eq!(parse_steps(Some("0")), Some(0));
}

/// 🔴 A typo must not read as an off switch. A measurement run that
/// silently marched nothing would credit the saving to whatever else
/// changed that day — the failure mode `KOOCH_SHADING_RATE` was written
/// to avoid.
#[test]
fn a_typo_says_nothing() {
    for raw in ["off", "", "-4", "sixteen", "8.0"] {
        assert_eq!(parse_steps(Some(raw)), None, "{raw:?} should say nothing");
    }
    assert_eq!(parse_steps(Some(" 8 ")), Some(8), "surrounding blanks");
}

/// Zero steps is the off switch, and it has to be reachable from the
/// settings rather than only from a light's flag — a project that
/// wants none of this should not pay a uniform read per light.
#[test]
fn zero_steps_is_expressible() {
    let settings = ContactShadowSettings {
        linear_steps: 0,
        ..Default::default()
    };
    let ubo = ContactShadowUbo::new(Mat4::IDENTITY, 0.1, &settings, 0);
    assert_eq!(ubo.linear_steps, 0);
}
