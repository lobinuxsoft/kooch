use super::*;
use glam::{Quat, Vec3};

#[test]
fn size_matches_shader() {
    // `inti_pbr.wgsl`'s IntiLight: vec3+f32, vec3+f32, vec3+u32,
    // f32, f32, u32, u32, f32, then three pad scalars — 80 B under
    // std430's vec3-aligns-to-16 rule. A mismatch here is the whole
    // struct read at the wrong stride.
    //
    // 🔴 The padding is why this is 80 and not 68: WGSL rounds the
    // struct up to its own alignment, Rust does not. Dropping the pad
    // fields would leave Rust writing at 68 and the shader reading at
    // 80, and every light past the first would be garbage.
    assert_eq!(std::mem::size_of::<GpuLight>(), 80);
    assert_eq!(std::mem::align_of::<GpuLight>(), 4);
}

#[test]
fn radius_scales_with_the_transform() {
    let world = Mat4::from_scale(Vec3::splat(3.0));
    let light = PointLight {
        radius: 0.5,
        ..Default::default()
    };
    assert_eq!(GpuLight::point(&light, world).radius, 1.5);
}

#[test]
fn a_directional_light_has_no_radius() {
    // There is no distance to a light with no position, so the
    // representative point has nothing to correct.
    let gpu = GpuLight::directional(&DirectionalLight::default(), Mat4::IDENTITY);
    assert_eq!(gpu.radius, 0.0);
}

#[test]
fn directional_takes_direction_from_the_transform() {
    let world = Mat4::from_quat(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
    let light = DirectionalLight::default();
    let gpu = GpuLight::directional(&light, world);
    // -Z rotated -90° about X points straight down.
    let dir = Vec3::from(gpu.direction);
    assert!(
        dir.abs_diff_eq(Vec3::NEG_Y, 1e-5),
        "expected -Y, got {dir:?}",
    );
}

#[test]
fn degenerate_transform_does_not_produce_nan() {
    let gpu = GpuLight::directional(&DirectionalLight::default(), Mat4::ZERO);
    assert!(Vec3::from(gpu.direction).is_finite());
}

#[test]
fn point_range_scales_with_the_transform_like_the_gizmo_does() {
    let world = Mat4::from_scale(Vec3::splat(2.0));
    let mut light = PointLight::default();
    light.range = 10.0;
    assert_eq!(GpuLight::point(&light, world).range, 20.0);
}

#[test]
fn spot_mad_is_one_inside_the_inner_cone_and_zero_outside_the_outer() {
    let (scale, offset) = spot_cone_mad(30.0, 45.0);
    let at = |deg: f32| (deg.to_radians().cos() * scale + offset).clamp(0.0, 1.0);
    assert!(
        (at(0.0) - 1.0).abs() < 1e-5,
        "axis should be full intensity"
    );
    assert!((at(30.0) - 1.0).abs() < 1e-5, "inner edge should be full");
    assert!(at(45.0).abs() < 1e-5, "outer edge should be dark");
    let mid = at(37.5);
    assert!(mid > 0.0 && mid < 1.0, "penumbra should ramp, got {mid}");
}

#[test]
fn inner_wider_than_outer_does_not_invert_the_cone() {
    let (scale, offset) = spot_cone_mad(60.0, 20.0);
    let at = |deg: f32| (deg.to_radians().cos() * scale + offset).clamp(0.0, 1.0);
    assert!((at(0.0) - 1.0).abs() < 1e-5);
    assert!(at(21.0).abs() < 1e-5);
    assert!(scale.is_finite() && offset.is_finite());
}

#[test]
fn coincident_angles_do_not_divide_by_zero() {
    let (scale, offset) = spot_cone_mad(45.0, 45.0);
    assert!(scale.is_finite(), "scale was {scale}");
    assert!(offset.is_finite(), "offset was {offset}");
}
