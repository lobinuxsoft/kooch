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
/// - `planes`: six pre-extracted world-space frustum planes packed
///   as `(normal, distance)` — used by the legacy sphere test
///   (`sphere_outside_frustum`) and the per-pass entries that
///   haven't migrated to AABB cull yet.
/// - `camera_position`: world-space camera position used by the
///   backface cone test.
/// - `lod_target_error_pixels` / `lod_error_to_pixel_factor`:
///   continuous-LOD selector knobs (#442).
/// - `debug_mode` / `debug_active`: editor-driven debug viz toggles
///   (#451 / #454.4).
/// - `view_proj`: clip-from-world matrix. Used by the AABB-vs-frustum
///   test (`aabb_outside_frustum_local`) the R64 atomic path now
///   shares with the Hi-Z 2-pass entry — both derive frustum planes
///   from `view_proj * inst.transform` to test AABBs in local space
///   without the world-envelope conservatism of an 8-corner box.
///   Sphere-bounds + plane test left silhouette holes on close-up
///   models at viewport edges (#488 documented this for the Hi-Z
///   path; the R64 path inherits the fix here).
///
/// Layout is 192 bytes — multiple of 16 to keep std140-friendly
/// alignment for the host-side `bytemuck::cast_slice` upload.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CullParams {
    pub planes: [[f32; 4]; 6],
    pub camera_position: [f32; 3],
    pub meshlet_count: u32,
    pub lod_target_error_pixels: f32,
    pub lod_error_to_pixel_factor: f32,
    /// Mirrors [`crate::meshlet::MeshletDebugMode`] discriminant.
    /// Most values are inert in the cull pass (the deferred shader
    /// is the consumer); but `OnlyLod0 = 8` and `OnlyRoots = 9`
    /// override the LOD selector so the cull emits only meshlets at
    /// a specific extreme of the chain — useful for visually
    /// auditing each chain layer in isolation.
    pub debug_mode: u32,
    /// `1` whenever the cull pass should record per-thread reject
    /// reasons into `MeshletCull::reject_reasons` (#454.4). The
    /// reject-overlay raster pass consumes those entries and paints
    /// rejection bounding boxes over the shaded image. Production
    /// rendering pays nothing — the cull-shader writes are gated to
    /// a single uniform compare and the SSBO stays untouched.
    pub debug_active: u32,
    pub view_proj: [[f32; 4]; 4],
}

impl CullParams {
    /// Builds with LOD selection effectively disabled — the pixel
    /// factor is `0`, so every meshlet's projected error is `0`,
    /// which (combined with the test `my_err <= threshold && parent_err > threshold`)
    /// makes only root-level meshlets pass. Legacy single-LOD assets
    /// have every meshlet at root, so behaviour stays identical.
    /// Multi-LOD assets need [`Self::with_lod`].
    pub fn new(view_projection: Mat4, camera_position: Vec3, meshlet_count: u32) -> Self {
        Self {
            planes: extract_frustum_planes(view_projection),
            camera_position: camera_position.to_array(),
            meshlet_count,
            lod_target_error_pixels: 1.0,
            lod_error_to_pixel_factor: 0.0,
            debug_mode: 0,
            debug_active: 0,
            view_proj: view_projection.to_cols_array_2d(),
        }
    }

    /// Sets the cull-side debug mode. Mirrors the deferred shader's
    /// `MeshletDebugMode` discriminant so a single resource drives
    /// both shading and cull behaviour.
    pub fn with_debug_mode(mut self, debug_mode: u32) -> Self {
        self.debug_mode = debug_mode;
        self
    }

    /// Toggles per-thread reject-reason recording on the
    /// `cs_cull_scene_pool_atomic` entry (#454.4). The overlay raster
    /// pass requires this to be `true`; everything else (including
    /// the deferred-shader colour overrides) leaves it at `false` so
    /// the cull hot path stays free of the SSBO write.
    pub fn with_debug_active(mut self, active: bool) -> Self {
        self.debug_active = active as u32;
        self
    }

    /// Configures the continuous-LOD selector with a non-zero
    /// projection factor. `proj_scale_y` is `1 / tan(fovy/2)`; get it
    /// from [`projection_scale_y`], which recovers it from a combined
    /// view-projection without depending on where the camera is looking.
    /// `viewport_height_pixels` is the destination framebuffer height in
    /// physical pixels.
    pub fn with_lod(
        mut self,
        viewport_height_pixels: f32,
        proj_scale_y: f32,
        lod_target_error_pixels: f32,
    ) -> Self {
        self.lod_target_error_pixels = lod_target_error_pixels;
        self.lod_error_to_pixel_factor = 0.5 * viewport_height_pixels * proj_scale_y;
        self
    }
}

/// Recovers the projection's vertical scale from a combined
/// view-projection matrix.
///
/// This is `1 / tan(fovy / 2)` for a perspective projection: how many
/// half-heights of clip space a unit of view-space Y becomes. It belongs
/// to the *projection*, so it must not change when the camera turns.
///
/// # Why the norm of a row and not one of its elements
///
/// The row of `view_projection` that produces `clip.y` is the projection's
/// `f` times row 1 of the view's rotation. A rotation's row is a **unit**
/// vector, so the row's length is exactly `f` no matter how the camera is
/// oriented — while any single component of it is `f` times a direction
/// cosine.
///
/// Reading one component was the bug: it happens to equal `f` when the
/// camera's up is the world's up, and decays to **zero** at 90° of roll or
/// looking straight up or down. A factor of zero disables the LOD selector
/// entirely (see `meshlet_cull/common.wgsl`), which keeps only root
/// meshlets — a sphere collapses to a blob and a cube to a spike. It
/// degrades continuously, so a moderate tilt silently lowered detail
/// everywhere rather than failing visibly.
///
/// Found by orbiting a `PointGravity`, not by any test: every previous
/// measurement was taken with a level camera.
///
/// Works for orthographic too, where the row's length is `2 / height`.
/// A view matrix carrying scale would fold that in — views are rotation
/// plus translation, so that does not arise.
pub fn projection_scale_y(view_projection: Mat4) -> f32 {
    // Row 1's xyz, read out of glam's column-major storage. The
    // translation lives in `w` and is deliberately excluded: it shifts
    // the image, it does not scale it.
    Vec3::new(
        view_projection.x_axis.y,
        view_projection.y_axis.y,
        view_projection.z_axis.y,
    )
    .length()
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

    // D3D / wgpu / Vulkan [0, 1] depth — works for BOTH standard-Z
    // (near→0, far→1) and reversed-Z (near→1, far→0). The two
    // formulas are derived directly from the clip-space constraints
    // `clip.z >= 0` (= row2) and `clip.w - clip.z >= 0` (= row3-row2);
    // both stay valid regardless of which plane is which under the
    // chosen depth orientation. The OpenGL formula `row3 + row2`
    // (which #488 had inherited) only cuts at `ndc.z >= -1`, so
    // points with `0 > ndc.z > -1` slipped through — invisible
    // under standard-Z but exposed by reversed-Z where beyond-far
    // points have negative ndc.z naturally.
    let raw = [
        row3 + row0, // left
        row3 - row0, // right
        row3 + row1, // bottom
        row3 - row1, // top
        row2,        // ndc.z >= 0 plane (call it "near" or "far"
        // depending on depth orientation — geometrically
        // it's the plane where the depth hits 0).
        row3 - row2, // ndc.z <= 1 plane.
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
    #[allow(unused_imports)]
    use super::super::asset::MeshletDescriptor;
    use super::*;
    use glam::Quat;

    #[test]
    fn cull_params_layout_is_pod() {
        // 6 planes (4 floats each) = 96 B, camera_position + meshlet_count = 16,
        // (lod_target, lod_factor, debug_mode, debug_active) = 16,
        // view_proj mat4 = 64. Total: 192 B.
        assert_eq!(std::mem::size_of::<CullParams>(), 192);
    }

    #[test]
    fn extracted_planes_are_normalised() {
        let proj = crate::projection::perspective_rh_reverse_z(
            60.0_f32.to_radians(),
            16.0 / 9.0,
            0.1,
            100.0,
        );
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
        let proj =
            crate::projection::perspective_rh_reverse_z(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        let planes = extract_frustum_planes(proj * view);

        // Sphere at world origin, radius 0.5 — should be visible
        // (camera 5 units away looking at origin).
        assert!(!sphere_outside_frustum(&planes, Vec3::ZERO, 0.5));
    }

    #[test]
    fn sphere_far_behind_camera_is_culled() {
        let proj =
            crate::projection::perspective_rh_reverse_z(90.0_f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::new(0.0, 0.0, 0.0), Vec3::Y);
        let planes = extract_frustum_planes(proj * view);

        // Sphere far behind the camera — outside near + far + side planes.
        let behind = Vec3::new(0.0, 0.0, 50.0);
        assert!(sphere_outside_frustum(&planes, behind, 0.5));
    }

    #[test]
    fn sphere_far_to_the_side_is_culled() {
        let proj =
            crate::projection::perspective_rh_reverse_z(60.0_f32.to_radians(), 1.0, 0.1, 100.0);
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
        let proj =
            crate::projection::perspective_rh_reverse_z(45.0_f32.to_radians(), 1.5, 1.0, 1000.0);
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
                Mat4::from_euler(glam::EulerRot::YXZ, 0.7, -0.9, 1.3)
                    * Mat4::from_translation(-eye),
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
        let wide =
            crate::projection::perspective_rh_reverse_z(90.0_f32.to_radians(), 1.0, 0.1, 1000.0);
        let narrow =
            crate::projection::perspective_rh_reverse_z(30.0_f32.to_radians(), 1.0, 0.1, 1000.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO, Vec3::Y);
        assert!(projection_scale_y(narrow * view) > projection_scale_y(wide * view));
    }
}
