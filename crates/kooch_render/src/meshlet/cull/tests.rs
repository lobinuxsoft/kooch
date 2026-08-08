/// With no far plane the `ndc.z >= 0` row degenerates to a
/// zero-length normal, and the extractor has to answer "cull
/// nothing" rather than divide by it. The shader already walks
/// five planes, so this one is skipped work rather than a wrong
/// test — but a NaN here would cull the whole scene.
#[test]
fn the_vanished_far_plane_culls_nothing_instead_of_producing_nan() {
    let proj =
        crate::projection::perspective_infinite_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1);
    let planes = super::extract_frustum_planes(proj);
    assert_eq!(
        planes[4],
        [0.0, 0.0, 0.0, 0.0],
        "the far plane should degenerate rather than carry a normal",
    );
    assert!(planes.iter().flatten().all(|c| c.is_finite()));
}

#[allow(unused_imports)]
use super::super::asset::MeshletDescriptor;
use super::*;
use glam::Quat;

#[test]
fn cull_params_layout_is_pod() {
    // 6 planes (4 floats each) = 96 B, camera_position + meshlet_count = 16,
    // (lod_target, lod_factor, debug_mode, debug_active) = 16,
    // (lod_orthographic + 3 pad) = 16, view_proj mat4 = 64. Total: 208 B.
    assert_eq!(std::mem::size_of::<CullParams>(), 208);
}

#[test]
fn extracted_planes_are_normalised() {
    let proj =
        crate::projection::perspective_rh_reverse_z(60.0_f32.to_radians(), 16.0 / 9.0, 0.1, 100.0);
    let view = Mat4::IDENTITY;
    let vp = proj * view;

    let planes = extract_frustum_planes(vp);
    for plane in &planes {
        let len = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
        assert!(
            (len - 1.0).abs() < 1e-3,
            "frustum plane normal should be unit length, got {len}",
        );
    }
}

#[test]
fn sphere_at_origin_inside_default_frustum() {
    let proj = crate::projection::perspective_rh_reverse_z(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
    let planes = extract_frustum_planes(proj * view);

    // Sphere at world origin, radius 0.5 — should be visible
    // (camera 5 units away looking at origin).
    assert!(!sphere_outside_frustum(&planes, Vec3::ZERO, 0.5));
}

#[test]
fn sphere_far_behind_camera_is_culled() {
    let proj = crate::projection::perspective_rh_reverse_z(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
    let planes = extract_frustum_planes(proj * view);

    // Sphere far behind the camera — outside near + far + side planes.
    let behind = Vec3::new(0.0, 0.0, 50.0);
    assert!(sphere_outside_frustum(&planes, behind, 0.5));
}

#[test]
fn sphere_far_to_the_side_is_culled() {
    let proj = crate::projection::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
    let view = Mat4::look_at_rh(Vec3::ZERO, -Vec3::Z, Vec3::Y);
    let planes = extract_frustum_planes(proj * view);

    // Sphere very far to the right — outside the right plane.
    let aside = Vec3::new(100.0, 0.0, -10.0);
    assert!(sphere_outside_frustum(&planes, aside, 0.5));
}

#[test]
fn cull_params_carries_meshlet_count_and_camera() {
    let vp = Mat4::IDENTITY;
    let cam = Vec3::new(2.0, 3.0, 4.0);
    let params = CullParams::new(vp, cam, 1234);
    assert_eq!(params.meshlet_count, 1234);
    assert_eq!(params.camera_position, [2.0, 3.0, 4.0]);
}

#[test]
fn camera_in_front_of_meshlet_is_not_culled() {
    // meshopt convention: `cone_axis` points along the meshlet's
    // average front-face normal. With `axis = +Z` the meshlet's
    // front faces look towards +Z, so a camera at +Z is IN FRONT
    // and must keep rendering. A camera at -Z is behind the
    // meshlet (backface side) and gets culled.
    let apex = Vec3::ZERO;
    let axis = Vec3::Z;
    let cutoff = 0.9;

    let cam_in_front = Vec3::new(0.0, 0.0, 5.0);
    assert!(
        !camera_in_backface_cone(apex, axis, cutoff, cam_in_front),
        "camera in front (+Z) must not be culled when front normals point +Z",
    );

    let cam_behind = Vec3::new(0.0, 0.0, -5.0);
    assert!(
        camera_in_backface_cone(apex, axis, cutoff, cam_behind),
        "camera behind (-Z) must be culled when front normals point +Z",
    );
}

#[test]
fn degenerate_cone_cutoff_disables_cull() {
    // meshopt sets cone_cutoff = 1.0 for divergent normal sets;
    // those meshlets must never be cone-culled regardless of cam pos.
    assert!(!camera_in_backface_cone(
        Vec3::ZERO,
        Vec3::Z,
        1.0,
        Vec3::new(0.0, 0.0, 5.0),
    ));
    assert!(!camera_in_backface_cone(
        Vec3::ZERO,
        Vec3::Z,
        1.0,
        Vec3::new(0.0, 0.0, -5.0),
    ));
}

#[test]
fn camera_at_apex_is_never_cone_culled() {
    // Length-zero view vector → cull test is undefined.
    // Conservative: keep the meshlet (camera is right on top of it).
    assert!(!camera_in_backface_cone(
        Vec3::new(1.0, 2.0, 3.0),
        Vec3::Z,
        0.5,
        Vec3::new(1.0, 2.0, 3.0),
    ));
}

#[test]
fn descriptor_cull_fields_are_addressable() {
    // Defensive: confirm MeshletDescriptor exposes the fields the
    // cull shader reads. If the layout drifts, this test fails
    // before the shader runs in production.
    let d = MeshletDescriptor::zeroed();
    let _ = d.bounds_center;
    let _ = d.bounding_radius;
    let _ = d.cone_apex;
    let _ = d.cone_axis;
    let _ = d.cone_cutoff;
}

#[test]
fn rotated_camera_still_normalises_planes() {
    let proj = crate::projection::perspective_rh_reverse_z(45.0_f32.to_radians(), 1.5, 1.0, 1000.0);
    let view =
        Mat4::from_rotation_translation(Quat::from_rotation_y(1.2), Vec3::new(10.0, 5.0, -3.0));
    let planes = extract_frustum_planes(proj * view);
    for p in &planes {
        let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        assert!((len - 1.0).abs() < 1e-3);
    }
}

/// The LOD factor belongs to the projection, so **no** camera
/// orientation may change it. The old code read
/// `view_proj.y_axis.y`, which is `f × cos(angle between the
/// camera's up and the world's)` — right for a level camera and
/// zero at 90° of roll or looking straight down. Zero switches the
/// LOD selector off, leaving only root meshlets: a sphere becomes a
/// blob.
///
/// Every case here fails against that formula, including the two
/// that silently return a *plausible but wrong* number rather than
/// zero.
#[test]
fn the_lod_factor_survives_any_camera_orientation() {
    use std::f32::consts::FRAC_PI_2;

    let fovy = 60.0_f32.to_radians();
    let expected = 1.0 / (fovy * 0.5).tan();
    let proj = crate::projection::perspective_rh_reverse_z(fovy, 16.0 / 9.0, 0.1, 1000.0);
    let eye = Vec3::new(3.0, 4.0, 5.0);

    let cases: [(&str, Mat4); 6] = [
        ("level", Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y)),
        // Rolled 90°: the camera's up is horizontal, so the element
        // the old code read is 0 and the selector shut down entirely.
        (
            "rolled 90°",
            Mat4::from_rotation_z(FRAC_PI_2) * Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y),
        ),
        ("upside down", Mat4::look_at_rh(eye, Vec3::ZERO, -Vec3::Y)),
        // Straight down — up ends up horizontal again. This is what
        // orbiting a PointGravity walks through.
        (
            "looking straight down",
            Mat4::look_at_rh(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, Vec3::Z),
        ),
        (
            "looking straight up",
            Mat4::look_at_rh(Vec3::new(0.0, -10.0, 0.0), Vec3::ZERO, Vec3::Z),
        ),
        (
            "arbitrary tilt",
            Mat4::from_euler(glam::EulerRot::YXZ, 0.7, -0.9, 1.3) * Mat4::from_translation(-eye),
        ),
    ];

    for (name, view) in cases {
        let got = projection_scale_y(proj * view);
        assert!(
            (got - expected).abs() < 1e-3,
            "{name}: projection scale drifted to {got}, expected {expected}"
        );
    }
}

/// Moving the camera must not change it either — the translation
/// lives in the row's `w`, which is excluded on purpose.
#[test]
fn the_lod_factor_survives_any_camera_position() {
    let fovy = 75.0_f32.to_radians();
    let expected = 1.0 / (fovy * 0.5).tan();
    let proj = crate::projection::perspective_rh_reverse_z(fovy, 1.0, 0.1, 1000.0);

    for eye in [
        Vec3::ZERO,
        Vec3::new(0.0, 0.0, -50.0),
        Vec3::new(1000.0, -2000.0, 3000.0),
    ] {
        let view = Mat4::from_translation(-eye);
        let got = projection_scale_y(proj * view);
        assert!(
            (got - expected).abs() < 1e-3,
            "at {eye:?}: got {got}, expected {expected}"
        );
    }
}

/// A narrower field of view concentrates more pixels on the same
/// object, so the same world-space error covers more of them — the
/// factor has to grow. Without this the test above would pass on a
/// function that returned a constant.
#[test]
fn a_narrower_field_of_view_raises_the_factor() {
    let wide = crate::projection::perspective_rh_reverse_z(90.0_f32.to_radians(), 1.0, 0.1, 1000.0);
    let narrow =
        crate::projection::perspective_rh_reverse_z(30.0_f32.to_radians(), 1.0, 0.1, 1000.0);
    let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
    assert!(projection_scale_y(narrow * view) > projection_scale_y(wide * view));
}
