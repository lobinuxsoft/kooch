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
/// Layout is 208 bytes — multiple of 16 to keep std140-friendly
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
    /// `1` when the view is orthographic, which changes the LOD test
    /// rather than tuning it.
    ///
    /// 🔴 Under perspective, a simplification error shrinks on screen
    /// with distance, so the selector divides by it. Under an
    /// orthographic projection it does not shrink at all: the screen
    /// error is the world error over the volume's world height, full
    /// stop. Dividing by a distance there makes the test vary across a
    /// shadow cascade for no physical reason, so neighbouring meshlets
    /// in one LOD group land on opposite sides of the threshold and the
    /// surface comes apart — which reads as "some meshlets do not cast".
    ///
    /// Bevy 0.19 branches on exactly this in `lod_error_is_imperceptible`
    /// (`if projection[3][3] == 1.0`), and this engine had no such
    /// branch because until shadows there was no orthographic view.
    pub lod_orthographic: u32,
    pub _pad_lod: [u32; 3],
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
            lod_orthographic: 0,
            _pad_lod: [0; 3],
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
        self.lod_orthographic = 0;
        self
    }

    /// The LOD selector for an orthographic view — a shadow cascade.
    ///
    /// `world_height` is how much world the volume spans vertically, and
    /// it is the whole of the relationship: an orthographic projection
    /// magnifies everything equally, so a simplification error covers
    /// `error / world_height` of the target no matter where it sits.
    /// There is no distance term to include, which is why this is a
    /// separate constructor rather than a different number fed to
    /// [`Self::with_lod`] — the shape of the test changes, not its
    /// tuning.
    pub fn with_orthographic_lod(
        mut self,
        world_height: f32,
        target_height_texels: f32,
        lod_target_error_pixels: f32,
    ) -> Self {
        self.lod_target_error_pixels = lod_target_error_pixels;
        self.lod_error_to_pixel_factor = target_height_texels / world_height.max(1e-6);
        self.lod_orthographic = 1;
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
mod tests;
