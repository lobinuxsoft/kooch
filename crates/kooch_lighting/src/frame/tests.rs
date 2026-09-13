use super::*;

/// 🔴 A Rust/WGSL size drift is no compile error, only `min_binding_size` refusing the pipeline.
#[test]
fn frame_size_matches_shader() {
    // 64 of header, four 96 B cascades, 16 of tail, four 96 B spot records with 16 of count, then
    // the point records, whose count took a spot pad word.
    const HEADER: usize = 64;
    const CASCADES: usize = 4 * 96;
    const TAIL: usize = 16;
    const SPOT_SHADOWS: usize = MAX_SPOT_SHADOWS * 96;
    const SPOT_TAIL: usize = 16;
    const POINT_SHADOWS: usize = MAX_POINT_SHADOWS * 16;
    // The grid's (#780) row, dimensions, factors and four words. ⚠️ Starts on a 16 B boundary;
    // `[f32; 4]` aligns to 4 in Rust but `vec4` to 16 in WGSL.
    const CLUSTERS: usize = 4 * 16;
    // #826's sample count: a whole 16 for one word, since the previous four closed their group.
    const SAMPLES: usize = 16;
    assert_eq!(
        std::mem::size_of::<IntiFrame>(),
        HEADER + CASCADES + TAIL + SPOT_SHADOWS + SPOT_TAIL + POINT_SHADOWS + CLUSTERS + SAMPLES,
    );
    // The literal, so a layout change is noticed: 1008 at 4 point shadows, 1456 at 32 (#849).
    assert_eq!(std::mem::size_of::<IntiFrame>(), 1456);
}

/// Array strides must be multiples of 16, or every cascade after the first is read mid-record.
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
    // 10 000 lux on a white Lambertian surface through the default exposure must land near 1.0, not
    // a white rectangle.
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
