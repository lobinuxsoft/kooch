//! Compute-side meshlet culling state.
//!
//! Owns the compute pipeline, the per-frame `CullParams` UBO, and the
//! output buffers (visible meshlet id list + atomic counter). Game code
//! calls [`MeshletCull::dispatch`] per frame inside the render encoder
//! after updating the camera.
//!
//! # Pipeline
//!
//! ```text
//! camera matrices  →  extract 6 frustum planes  →  CullParams UBO
//!                                                       │
//!                                                       ▼
//!                                meshlet bind group (#117 PR-2)
//!                                                       │
//!                                                       ▼
//!                                       meshlet_cull.wgsl
//!                              (one thread per meshlet, frustum test)
//!                                                       │
//!                                                       ▼
//!                          visible_meshlets[] + visible_count (atomic)
//! ```
//!
//! `visible_count` doubles as the `instance_count` field of an indirect
//! draw args buffer in the next PR (#117 PR-4).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3, Vec4};

/// Per-frame culling parameters uploaded to the compute shader.
///
/// - `planes`: six frustum planes packed as `(normal, distance)` —
///   plane equation `dot(normal, p) + distance >= 0` means inside.
///   Visibility against the frustum is the AND of all six.
/// - `camera_position`: world-space camera position used by the
///   backface cone test. The shader rejects a meshlet when
///   `dot(normalize(camera - cone_apex), cone_axis) >= cone_cutoff`
///   (the camera lies in the meshlet's back-facing half-space).
///
/// Layout is 128 bytes — multiple of 16 to keep std140-friendly
/// alignment for the host-side `bytemuck::cast_slice` upload.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CullParams {
    pub planes: [[f32; 4]; 6],
    pub camera_position: [f32; 3],
    pub meshlet_count: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
    pub _pad3: u32,
}

impl CullParams {
    pub fn new(view_projection: Mat4, camera_position: Vec3, meshlet_count: u32) -> Self {
        Self {
            planes: extract_frustum_planes(view_projection),
            camera_position: camera_position.to_array(),
            meshlet_count,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
            _pad3: 0,
        }
    }
}

/// CPU mirror of the WGSL backface cone test. Returns `true` when the
/// meshlet is fully back-facing relative to the camera and can be
/// skipped.
///
/// `cone_axis` follows meshopt's convention: it points along the
/// meshlet's average front-face normal. The test forms the
/// camera-to-apex vector and accepts the cull when its alignment with
/// the axis exceeds `cone_cutoff` — that is the sign Bevy / UE5 / the
/// meshoptimizer documentation use.
///
/// `cone_cutoff == 1.0` is the "no cull" sentinel that
/// `meshopt::compute_meshlet_bounds` returns for degenerate /
/// divergent normal sets — those meshlets must always survive cone
/// cull.
pub fn camera_in_backface_cone(
    cone_apex: Vec3,
    cone_axis: Vec3,
    cone_cutoff: f32,
    camera_position: Vec3,
) -> bool {
    if cone_cutoff >= 1.0 {
        return false;
    }
    let to_apex = cone_apex - camera_position;
    let len = to_apex.length();
    if len == 0.0 {
        return false;
    }
    let view = to_apex / len;
    view.dot(cone_axis) >= cone_cutoff
}

/// Extracts six frustum planes from a combined `view_projection` matrix.
///
/// Standard derivation: each plane is `row3 ± row_n` of the matrix.
/// Returned normalised so distance comparisons are world-space metric.
///
/// Order: left, right, bottom, top, near, far.
pub fn extract_frustum_planes(vp: Mat4) -> [[f32; 4]; 6] {
    let m = vp.to_cols_array_2d();
    // glam to_cols_array_2d returns column-major, so we read rows by index.
    let row = |i: usize| Vec4::new(m[0][i], m[1][i], m[2][i], m[3][i]);
    let row0 = row(0);
    let row1 = row(1);
    let row2 = row(2);
    let row3 = row(3);

    let raw = [
        row3 + row0, // left
        row3 - row0, // right
        row3 + row1, // bottom
        row3 - row1, // top
        row3 + row2, // near (assumes wgpu/D3D-style [0, 1] depth)
        row3 - row2, // far
    ];

    let mut out = [[0.0f32; 4]; 6];
    for i in 0..6 {
        let plane = raw[i];
        let n = Vec3::new(plane.x, plane.y, plane.z);
        let len = n.length();
        if len > 0.0 {
            out[i] = [plane.x / len, plane.y / len, plane.z / len, plane.w / len];
        } else {
            out[i] = [0.0, 0.0, 0.0, 0.0];
        }
    }
    out
}

/// Returns `true` if the sphere is fully OUTSIDE any of the planes.
/// Pure CPU — used for tests + a CPU fallback path; the shader does
/// the same math on GPU.
pub fn sphere_outside_frustum(planes: &[[f32; 4]; 6], center: Vec3, radius: f32) -> bool {
    for plane in planes {
        let normal = Vec3::new(plane[0], plane[1], plane[2]);
        let signed_dist = normal.dot(center) + plane[3];
        if signed_dist < -radius {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use super::super::asset::MeshletDescriptor;
    use glam::Quat;

    #[test]
    fn cull_params_layout_is_pod() {
        // 6 planes (4 floats each) = 96 bytes, camera_position + meshlet_count = 16,
        // four u32 pads = 16. Total: 128 bytes.
        assert_eq!(std::mem::size_of::<CullParams>(), 128);
    }

    #[test]
    fn extracted_planes_are_normalised() {
        let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 16.0 / 9.0, 0.1, 100.0);
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
        let proj = Mat4::perspective_rh(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::ZERO,
            Vec3::Y,
        );
        let planes = extract_frustum_planes(proj * view);

        // Sphere at world origin, radius 0.5 — should be visible
        // (camera 5 units away looking at origin).
        assert!(!sphere_outside_frustum(&planes, Vec3::ZERO, 0.5));
    }

    #[test]
    fn sphere_far_behind_camera_is_culled() {
        let proj = Mat4::perspective_rh(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(
            Vec3::new(0.0, 0.0, 5.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::Y,
        );
        let planes = extract_frustum_planes(proj * view);

        // Sphere far behind the camera — outside near + far + side planes.
        let behind = Vec3::new(0.0, 0.0, 50.0);
        assert!(sphere_outside_frustum(&planes, behind, 0.5));
    }

    #[test]
    fn sphere_far_to_the_side_is_culled() {
        let proj = Mat4::perspective_rh(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
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
        let proj = Mat4::perspective_rh(45.0_f32.to_radians(), 1.5, 1.0, 1000.0);
        let view = Mat4::from_rotation_translation(
            Quat::from_rotation_y(1.2),
            Vec3::new(10.0, 5.0, -3.0),
        );
        let planes = extract_frustum_planes(proj * view);
        for p in &planes {
            let len = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-3);
        }
    }
}
