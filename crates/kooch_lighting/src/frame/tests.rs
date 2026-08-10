use super::*;

/// 🔴 A uniform whose Rust size drifts from its WGSL mirror is not a
/// compile error anywhere: it surfaces as `min_binding_size` refusing
/// the pipeline, which reads as a bindings bug. It is how the `vec3<u32>`
/// pad that grew every cascade by 16 bytes was found, and only because
/// this number is written down.
#[test]
fn frame_size_matches_shader() {
    // 64 of header, four 96-byte cascades, 16 of tail, then #777's four
    // 96-byte spot-shadow records and their own 16 of count and pad.
    const HEADER: usize = 64;
    const CASCADES: usize = 4 * 96;
    const TAIL: usize = 16;
    const SPOT_SHADOWS: usize = MAX_SPOT_SHADOWS * 96;
    const SPOT_TAIL: usize = 16;
    assert_eq!(
        std::mem::size_of::<IntiFrame>(),
        HEADER + CASCADES + TAIL + SPOT_SHADOWS + SPOT_TAIL,
    );
    assert_eq!(std::mem::size_of::<IntiFrame>(), 864);
}

/// std140/std430 require an array's element stride to be a multiple
/// of 16. At 92 or 100 bytes every cascade after the first would be
/// read from the middle of the previous one, and the symptom is
/// shadows in the wrong place rather than a validation error.
#[test]
fn cascade_stride_is_sixteen_byte_aligned() {
    assert_eq!(std::mem::size_of::<GpuCascade>(), 96);
    assert_eq!(std::mem::size_of::<GpuCascade>() % 16, 0);
}

#[test]
fn shadows_are_off_until_cascades_are_attached() {
    let frame = IntiFrame::new(
        &AmbientLight::default(),
        &Exposure::default(),
        Vec3::ZERO,
        0,
    );
    assert_eq!(frame.shadows_enabled, 0);
    let lit = frame.with_shadows(Vec3::NEG_Z, [GpuCascade::default(); 4], 0.1, 0.03);
    assert_eq!(lit.shadows_enabled, 1);
}

#[test]
fn a_degenerate_forward_falls_back_rather_than_producing_nan() {
    let frame =
        IntiFrame::default().with_shadows(Vec3::ZERO, [GpuCascade::default(); 4], 0.1, 0.03);
    assert!(Vec3::from(frame.camera_forward).is_finite());
}

#[test]
fn default_exposure_brings_a_default_sun_into_range() {
    // 10 000 lux × exposure, through a Lambertian white surface
    // facing the light, must land near 1.0 rather than clipping by
    // an order of magnitude. This is the assertion that catches
    // "the whole scene is a white rectangle" before a smoke test
    // has to.
    let peak = 10_000.0 * Exposure::default().multiplier() / std::f32::consts::PI;
    assert!(
        (0.5..8.0).contains(&peak),
        "peak diffuse response was {peak}, tonemapping cannot rescue that",
    );
}

#[test]
fn exposure_is_monotonic_in_ev100() {
    assert!(Exposure { ev100: 8.0 }.multiplier() > Exposure { ev100: 12.0 }.multiplier());
}

/// Swapping one control for the other must not change the picture
/// until someone turns a dial.
#[test]
fn the_two_exposure_controls_agree_by_construction() {
    assert_eq!(
        Exposure::default().ev100,
        PhysicalCamera::default().ev100(),
        "the default exposure and the default camera describe the same light",
    );
}

/// The presets are named after real situations, so the arithmetic
/// has to land where photography says it does — otherwise the names
/// are decoration.
#[test]
fn the_presets_land_on_their_photographic_values() {
    let sunny = PhysicalCamera::sunny().ev100();
    let indoor = PhysicalCamera::indoor().ev100();
    assert!(
        (sunny - 15.0).abs() < 0.2,
        "sunny 16 is EV100 15, got {sunny}",
    );
    assert!(
        (indoor - 7.0).abs() < 0.2,
        "a lit interior is EV100 7, got {indoor}",
    );
}

/// Kept close to Bevy's 9.7 so a scene authored against their
/// numbers reads the same here. Their value is not "sunny 16"
/// despite how it is usually described — they matched Blender.
#[test]
fn the_default_stays_within_a_stop_of_bevys() {
    assert!(
        (Exposure::default().ev100 - 9.7).abs() < 1.0,
        "drifted to {}, and a scene ported from Bevy will not match",
        Exposure::default().ev100,
    );
}

#[test]
fn opening_the_aperture_brightens_the_image() {
    let wide = Exposure::from_physical(PhysicalCamera {
        aperture_f_stops: 1.4,
        ..Default::default()
    });
    let narrow = Exposure::from_physical(PhysicalCamera::default());
    assert!(
        wide.multiplier() > narrow.multiplier(),
        "a wider aperture has to let in more light, or the control lies",
    );
}

/// The gap that makes a physically-correct bulb look like nothing.
#[test]
fn the_indoor_preset_is_several_stops_brighter_than_sunlight() {
    let stops = PhysicalCamera::sunny().ev100() - PhysicalCamera::indoor().ev100();
    assert!(
        (6.0..10.0).contains(&stops),
        "indoor is {stops} stops from sunlight, which is not the gap it is for",
    );
}

#[test]
fn degenerate_camera_settings_do_not_produce_nan() {
    let broken = PhysicalCamera {
        aperture_f_stops: 0.0,
        shutter_speed_s: 0.0,
        sensitivity_iso: 0.0,
    };
    assert!(broken.ev100().is_finite(), "got {}", broken.ev100());
}
