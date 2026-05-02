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
/// Six frustum planes packed as `(normal, distance)` — plane equation
/// `dot(normal, p) + distance >= 0` means inside. Visibility is the
/// AND of all six.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CullParams {
    pub planes: [[f32; 4]; 6],
    pub meshlet_count: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

impl CullParams {
    pub fn new(view_projection: Mat4, meshlet_count: u32) -> Self {
        Self {
            planes: extract_frustum_planes(view_projection),
            meshlet_count,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        }
    }
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
        // 6 planes (4 floats each) = 96 bytes, plus meshlet_count + 3 pads = 16
        // Total: 112 bytes
        assert_eq!(std::mem::size_of::<CullParams>(), 112);
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
    fn cull_params_carries_meshlet_count() {
        let vp = Mat4::IDENTITY;
        let params = CullParams::new(vp, 1234);
        assert_eq!(params.meshlet_count, 1234);
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
