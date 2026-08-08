use super::*;
use crate::perspective_rh_reverse_z;

fn camera(pitch: f32, yaw: f32, position: Vec3) -> Mat4 {
    let rotation = glam::Quat::from_euler(glam::EulerRot::YXZ, yaw, pitch, 0.0);
    let to_world = Mat4::from_rotation_translation(rotation, position);
    perspective_rh_reverse_z(60.0_f32.to_radians(), 1.6, 0.1, 500.0) * to_world.inverse()
}

#[test]
fn splits_cover_the_range_and_increase() {
    let splits = split_distances(10.0, 1000.0);
    assert!(
        splits.windows(2).all(|w| w[0] < w[1]),
        "splits must increase: {splits:?}",
    );
    assert!(
        (splits[CASCADE_COUNT - 1] - 1000.0).abs() < 1.0,
        "the last split is the far plane, got {}",
        splits[CASCADE_COUNT - 1],
    );
}

/// The first cascade covers exactly what it was asked to, rather
/// than a share of the range derived from the lens. That is the
/// difference between a scheme an author can reason about and one
/// that changes when someone edits the camera's near plane.
#[test]
fn the_first_cascade_ends_where_it_was_told_to() {
    let splits = split_distances(10.0, 1000.0);
    assert!(
        (splits[0] - 10.0).abs() < 1e-3,
        "first split at {}",
        splits[0]
    );
}

/// Each cascade covers the same ratio of distance as the last, which
/// is what makes a texel subtend roughly one screen angle in all
/// four rather than four different ones.
#[test]
fn every_cascade_covers_the_same_ratio_as_the_last() {
    let splits = split_distances(10.0, 1000.0);
    let ratio = splits[1] / splits[0];
    for w in splits.windows(2) {
        assert!(
            (w[1] / w[0] - ratio).abs() < 1e-3,
            "ratios differ across the chain: {splits:?}",
        );
    }
}

/// 🔴 Consecutive cascades must OVERLAP, not merely touch.
///
/// The shading pass blends the two across the last
/// `CASCADE_BLEND_FRACTION` of a split. If the next cascade's volume
/// starts exactly where this one ends, every point in that band is
/// outside it, the sample comes back "fully lit", and the blend
/// paints a pale stripe across every shadow crossing the boundary.
#[test]
fn each_cascade_starts_inside_the_previous_one() {
    let cascades = build_cascades(
        camera(0.0, 0.0, Vec3::ZERO),
        Vec3::new(0.3, -1.0, 0.2),
        0.1,
        100.0,
        10.0,
        2048,
        0.0,
    );
    // The volume is a square of side `texel_world_size * size`
    // centred on the slice, so a cascade reaching back over the
    // blend band shows up as its width covering more than its own
    // split range.
    for i in 1..CASCADE_COUNT {
        let previous_far = cascades[i - 1].far_depth;
        let this_far = cascades[i].far_depth;
        let band = previous_far * CASCADE_BLEND_FRACTION;
        let covered = cascades[i].texel_world_size * 2048.0;
        assert!(
            covered >= this_far - (previous_far - band),
            "cascade {i} spans {covered} world units but has to reach \
                 back {} to cover the blend band",
            this_far - (previous_far - band),
        );
    }
}

/// 🔴 Property one. Rotating the camera in place must not change how
/// much world a shadow texel covers. If it does, every shadow edge
/// crawls as you look around, and no filter hides it.
#[test]
fn texel_size_is_invariant_to_camera_rotation() {
    let light = Vec3::new(-0.3, -1.0, -0.2);
    let at = Vec3::new(5.0, 2.0, -3.0);
    let mut sizes = Vec::new();
    for yaw_deg in [0.0f32, 17.0, 45.0, 90.0, 133.0, 180.0, 271.0] {
        let c = camera(0.0, yaw_deg.to_radians(), at);
        let cascades = build_cascades(c, light, 0.1, 500.0, 10.0, 2048, 4.0);
        sizes.push(cascades[0].texel_world_size);
    }
    let min = sizes.iter().copied().fold(f32::INFINITY, f32::min);
    let max = sizes.iter().copied().fold(0.0f32, f32::max);
    assert!(
        (max - min) / max < 1e-3,
        "texel size varies with yaw: {sizes:?} — the slice is being \
             bounded by a box, not a sphere",
    );
}

/// Same for pitch, which is the axis a bounding box distorts worst.
#[test]
fn texel_size_is_invariant_to_camera_pitch() {
    let light = Vec3::NEG_Y;
    let mut sizes = Vec::new();
    for pitch_deg in [-80.0f32, -40.0, 0.0, 40.0, 80.0] {
        let c = camera(pitch_deg.to_radians(), 0.0, Vec3::ZERO);
        sizes.push(build_cascades(c, light, 0.1, 500.0, 10.0, 2048, 4.0)[1].texel_world_size);
    }
    let min = sizes.iter().copied().fold(f32::INFINITY, f32::min);
    let max = sizes.iter().copied().fold(0.0f32, f32::max);
    assert!(
        (max - min) / max < 1e-3,
        "texel size varies with pitch: {sizes:?}"
    );
}

/// 🔴 Property two. Translating the camera by less than a texel must
/// not move the shadow projection at all — it snaps, or it swims.
#[test]
fn sub_texel_camera_movement_does_not_move_the_projection() {
    let light = Vec3::new(0.0, -1.0, -0.35);
    let base = camera(0.0, 0.0, Vec3::ZERO);
    let cascades = build_cascades(base, light, 0.1, 500.0, 10.0, 2048, 4.0);
    let texel = cascades[0].texel_world_size;

    // A hundredth of a texel: far below the quantisation step.
    let nudged = camera(0.0, 0.0, Vec3::new(texel * 0.01, 0.0, 0.0));
    let moved = build_cascades(nudged, light, 0.1, 500.0, 10.0, 2048, 4.0);

    let a = cascades[0].view_proj.to_cols_array();
    let b = moved[0].view_proj.to_cols_array();
    let drift = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max);
    assert!(
        drift < 1e-4,
        "the projection moved by {drift} for a sub-texel camera nudge — \
             the centre is not snapped to the texel grid",
    );
}

/// And a movement of many texels must move it, or the "snap" is a
/// freeze and the cascade stops following the camera.
#[test]
fn large_camera_movement_does_move_the_projection() {
    let light = Vec3::new(0.0, -1.0, -0.35);
    let base = camera(0.0, 0.0, Vec3::ZERO);
    let cascades = build_cascades(base, light, 0.1, 500.0, 10.0, 2048, 4.0);
    let far = camera(
        0.0,
        0.0,
        Vec3::new(cascades[0].texel_world_size * 500.0, 0.0, 0.0),
    );
    let moved = build_cascades(far, light, 0.1, 500.0, 10.0, 2048, 4.0);
    assert_ne!(cascades[0].view_proj, moved[0].view_proj);
}

#[test]
fn cascades_are_ordered_near_to_far_and_grow() {
    let cascades = build_cascades(
        camera(0.0, 0.0, Vec3::ZERO),
        Vec3::NEG_Y,
        0.1,
        500.0,
        10.0,
        2048,
        4.0,
    );
    for pair in cascades.windows(2) {
        assert!(pair[0].far_depth < pair[1].far_depth);
        assert!(
            pair[0].texel_world_size < pair[1].texel_world_size,
            "a farther cascade must cover more world per texel, or the \
                 split scheme is upside down",
        );
    }
}

/// A light pointing straight down is the case that picks a degenerate
/// up vector, and it is also the most common light in any scene.
#[test]
fn a_straight_down_light_produces_finite_matrices() {
    let cascades = build_cascades(
        camera(0.0, 0.0, Vec3::ZERO),
        Vec3::NEG_Y,
        0.1,
        500.0,
        10.0,
        2048,
        4.0,
    );
    for (i, c) in cascades.iter().enumerate() {
        assert!(
            c.view_proj.to_cols_array().iter().all(|v| v.is_finite()),
            "cascade {i} has a non-finite matrix",
        );
    }
}

#[test]
fn a_degenerate_light_direction_falls_back_rather_than_producing_nan() {
    let cascades = build_cascades(
        camera(0.0, 0.0, Vec3::ZERO),
        Vec3::ZERO,
        0.1,
        500.0,
        10.0,
        2048,
        4.0,
    );
    assert!(
        cascades[0]
            .view_proj
            .to_cols_array()
            .iter()
            .all(|v| v.is_finite())
    );
}

/// Reversed-Z: a point at the light's near plane must land at ndc.z
/// = 1 and one at the far plane at 0. Getting this backwards fills
/// the atlas with the farthest surface instead of the nearest, and
/// every shadow inverts.
#[test]
fn orthographic_is_reversed_z() {
    let p = orthographic_rh_reverse_z(-1.0, 1.0, -1.0, 1.0, 0.0, 100.0);
    // Right-handed: the camera looks down -Z, so the near plane is
    // at z = 0 and the far plane at z = -100.
    let near = p * glam::Vec4::new(0.0, 0.0, 0.0, 1.0);
    let far = p * glam::Vec4::new(0.0, 0.0, -100.0, 1.0);
    assert!(
        (near.z / near.w - 1.0).abs() < 1e-5,
        "near mapped to {}",
        near.z / near.w
    );
    assert!(
        (far.z / far.w).abs() < 1e-5,
        "far mapped to {}",
        far.z / far.w
    );
}
