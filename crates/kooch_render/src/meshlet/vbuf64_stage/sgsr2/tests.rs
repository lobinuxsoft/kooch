use super::*;

/// The transliteration has to survive naga before it can be judged by
/// eye, and a WGSL error at pipeline-creation time is a panic inside a
/// GPU test that says nothing about which line is wrong.
#[test]
fn the_convert_shader_validates() {
    let module =
        naga::front::wgsl::parse_str(CONVERT_SOURCE).expect("sgsr2 convert shader should parse");
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    );
    validator
        .validate(&module)
        .expect("sgsr2 convert shader should validate");
}

/// 1:1 is the identity, and it is the configuration the port is
/// validated at — see the module header. A `scale_ratio` that is not
/// exactly `(1, 1)` there would widen the variance box for an upscale
/// that is not happening, and the comparison against the existing
/// resolve would be measuring the wrong thing.
#[test]
fn one_to_one_is_the_identity() {
    assert_eq!(scale_ratio((1280, 720), (1280, 720)), [1.0, 1.0]);
}

/// 🔴 The second component is the CUBE of the AREA ratio, capped at 20.
///
/// Both halves of that are upstream's and neither is obvious. Cubing
/// the linear ratio instead — the plausible misreading — gives 3.4 at
/// 1.5x where the real value is 11.4, so the variance box stays tight
/// exactly where the reconstruction has the fewest samples to build it
/// from, and the result flickers.
#[test]
fn the_box_widens_by_the_area_cubed() {
    let [linear, box_scale] = scale_ratio((1280, 720), (1920, 1080));
    assert!((linear - 1.5).abs() < 1e-6, "linear ratio was {linear}");
    // (1.5^2)^3 = 2.25^3 = 11.390625
    assert!(
        (box_scale - 11.390625).abs() < 1e-4,
        "box scale was {box_scale}, expected the area ratio cubed",
    );

    // 2x squares to 4 and cubes to 64, which is where their cap bites.
    let [_, capped] = scale_ratio((960, 540), (1920, 1080));
    assert_eq!(capped, 20.0);
}

/// `fov_k` is `tan(fov_horizontal / 2)`, however it is spelled.
///
/// Upstream computes it from the VERTICAL fov and the aspect ratio, and
/// this asserts the two spellings agree — because the recovered formula
/// is the one thing here taken from a third-party port rather than from
/// Qualcomm, and this is what makes that recovery checkable.
#[test]
fn fov_k_is_the_horizontal_half_tangent() {
    let fov_vertical = 60.0f32.to_radians();
    let aspect = 16.0 / 9.0;

    let from_vertical = fov_k(fov_vertical, aspect);
    let fov_horizontal = 2.0 * ((fov_vertical * 0.5).tan() * aspect).atan();
    let from_horizontal = (fov_horizontal * 0.5).tan();

    assert!(
        (from_vertical - from_horizontal).abs() < 1e-5,
        "{from_vertical} vs {from_horizontal}",
    );
}

/// A degenerate size must not divide by zero: a minimised window would
/// otherwise hand the shader an infinite ratio, and an infinity written
/// into the variance box stays there.
#[test]
fn a_zero_size_stays_finite() {
    let [linear, box_scale] = scale_ratio((0, 0), (1920, 1080));
    assert!(linear.is_finite() && box_scale.is_finite());
    assert_eq!(box_scale, 20.0);
}
