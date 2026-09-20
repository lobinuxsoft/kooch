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
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CullParams {
    pub planes: [[f32; 4]; 6],
    pub camera_position: [f32; 3],
    pub meshlet_count: u32,
    pub lod_target_error_pixels: f32,
    pub lod_error_to_pixel_factor: f32,
    /// Mirrors [`crate::meshlet::MeshletDebugMode`] discriminant.
    pub debug_mode: u32,
    /// `1` whenever the cull pass should record per-thread reject reasons into
    /// `MeshletCull::reject_reasons` (#454.4). The reject-overlay raster pass consumes those
    /// entries and paints rejection bounding boxes over the shaded image.
    pub debug_active: u32,
    /// `1` when the view is orthographic, which changes the LOD test rather than tuning it.
    pub lod_orthographic: u32,
    /// Projected radius, in pixels, under which an INSTANCE is rejected before it ever becomes
    /// meshlets (#1002). `0` = off, which is what ships.
    pub min_screen_pixels: f32,
    /// Instances whose `flags` share a bit with this are skipped: a shadow view sets
    /// [`INSTANCE_CASTS_NO_SHADOW`](crate::meshlet::scene::INSTANCE_CASTS_NO_SHADOW) (#452).
    pub skip_flags: u32,
    /// Layers this view draws (#1219). An instance is rejected when its own `layers` share no bit
    /// with it; `u32::MAX` is every layer, which is what a view that was never told draws.
    pub culling_mask: u32,
    pub view_proj: [[f32; 4]; 4],
}

impl CullParams {
    /// Builds with LOD selection effectively disabled — the pixel factor is `0`, so every meshlet's
    /// projected error is `0`, which (combined with the test `my_err <= threshold && parent_err >
    /// threshold`) makes only root-level meshlets pass.
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
            min_screen_pixels: 0.0,
            skip_flags: 0,
            culling_mask: u32::MAX,
            view_proj: view_projection.to_cols_array_2d(),
        }
    }

    /// Rejects an instance whose bounding sphere covers fewer than
    /// `pixels` on screen, before it is expanded into meshlets
    /// (#1002). `0` disables the test.
    pub fn with_min_screen_pixels(mut self, pixels: f32) -> Self {
        self.min_screen_pixels = pixels.max(0.0);
        self
    }

    /// Sets the cull-side debug mode. Mirrors the deferred shader's
    /// `MeshletDebugMode` discriminant so a single resource drives
    /// both shading and cull behaviour.
    pub fn with_debug_mode(mut self, debug_mode: u32) -> Self {
        self.debug_mode = debug_mode;
        self
    }

    /// Toggles per-thread reject-reason recording on the `cs_cull_scene_pool_atomic` entry
    /// (#454.4).
    pub fn with_debug_active(mut self, active: bool) -> Self {
        self.debug_active = active as u32;
        self
    }

    /// Configures the continuous-LOD selector with a non-zero projection factor. `proj_scale_y` is
    /// `1 / tan(fovy/2)`; get it from [`projection_scale_y`], which recovers it from a combined
    /// view-projection without depending on where the camera is looking.
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
    /// A shadow view's parameters: as [`Self::new`], skipping the instances that cast no shadow
    /// (#452).
    pub fn shadow(view_projection: Mat4, camera_position: Vec3, meshlet_count: u32) -> Self {
        Self {
            skip_flags: crate::meshlet::scene::INSTANCE_CASTS_NO_SHADOW,
            ..Self::new(view_projection, camera_position, meshlet_count)
        }
    }

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

    /// Draws only the layers in `mask` (#1219). `u32::MAX` draws every layer.
    pub fn with_culling_mask(mut self, mask: u32) -> Self {
        self.culling_mask = mask;
        self
    }
}

/// Recovers the projection's vertical scale from a combined view-projection matrix.
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

/// CPU mirror of the WGSL backface cone test. Returns `true` when the meshlet is fully back-facing
/// relative to the camera and can be skipped.
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
pub fn extract_frustum_planes(vp: Mat4) -> [[f32; 4]; 6] {
    let m = vp.to_cols_array_2d();
    // glam to_cols_array_2d returns column-major, so we read rows by index.
    let row = |i: usize| Vec4::new(m[0][i], m[1][i], m[2][i], m[3][i]);
    let row0 = row(0);
    let row1 = row(1);
    let row2 = row(2);
    let row3 = row(3);

    // D3D / wgpu / Vulkan [0, 1] depth — works for BOTH standard-Z (near→0, far→1) and reversed-Z
    // (near→1, far→0).
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
