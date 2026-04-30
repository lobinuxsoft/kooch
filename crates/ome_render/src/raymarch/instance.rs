//! GPU primitive + scene metadata layouts for the unified ray-march pipeline.
//!
//! The byte layout of [`SdfPrimitive`] matches the WGSL `SdfPrimitive`
//! struct in `raymarch_main.wgsl` under std430 storage-buffer rules.
//! CSG composition lives in a separate token SSBO (see
//! [`super::csg_tree`]) — primitives carry only their own intrinsic
//! geometric data.

use bytemuck::{Pod, Zeroable};

/// Primitive type tags. Must match the `switch` in
/// `raymarch_main.wgsl::eval_primitive`.
pub(super) const TYPE_SPHERE: u32 = 0;
pub(super) const TYPE_BOX: u32 = 1;
pub(super) const TYPE_CAPSULE: u32 = 2;
pub(super) const TYPE_CYLINDER: u32 = 3;
pub(super) const TYPE_TORUS: u32 = 4;
pub(super) const TYPE_PLANE: u32 = 5;

/// Matches `CameraUniforms` in the WGSL shader.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub(super) struct CameraUniforms {
    pub view: [[f32; 4]; 4],
    pub projection: [[f32; 4]; 4],
    pub inverse_view: [[f32; 4]; 4],
    pub inverse_projection: [[f32; 4]; 4],
    pub position: [f32; 3],
    pub _pad0: f32,
}

/// Matches `RayMarchParams` in the WGSL shader.
///
/// The hit test uses an **adaptive epsilon**: a ray is considered on the
/// surface when the signed distance is below
/// `surface_threshold + epsilon_factor * distance_traveled`. The
/// distance-proportional term approximates the pixel-cone footprint,
/// avoiding shimmer on far surfaces (where a pixel covers many world
/// units) and saving iterations in regions where sub-mm precision
/// doesn't matter.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug)]
pub struct RayMarchParams {
    pub max_steps: u32,
    pub max_distance: f32,
    /// Base hit threshold at distance zero. Sets the precision for
    /// close-up geometry.
    pub surface_threshold: f32,
    /// Linear coefficient scaling the threshold with distance travelled.
    /// A perfect pixel-cone match would be `1 / viewport_height`; `1e-3`
    /// is a reasonable default for 720p-1440p viewports.
    pub epsilon_factor: f32,
}

impl Default for RayMarchParams {
    fn default() -> Self {
        Self {
            // 256 (vs the prior 128) lets sphere-tracing converge through
            // concave necks and grazing silhouettes that previously hit the
            // budget. ~2x cost in the worst case but the test scene stays
            // well under render budget on RDNA4. A future engine-settings
            // panel will let users tune this per-scene at runtime.
            max_steps: 256,
            max_distance: 100.0,
            surface_threshold: 0.001,
            // Adaptive term: at t=10 threshold is 0.011; at t=100 it's
            // 0.101. Eliminates far-surface shimmer without relaxing the
            // close-up precision defined above.
            epsilon_factor: 0.001,
        }
    }
}

/// Per-entity SDF primitive (64 bytes).
///
/// Field offsets match the WGSL struct byte-for-byte:
/// - `position` (vec3 at 0) + `type_tag` (u32 at 12) fill the first 16-byte slot.
/// - `rotation` (vec4 at 16) is naturally 16-aligned.
/// - `scale` (vec3 at 32) + `_pad0` (f32 at 44) fill the next 16-byte slot.
/// - `params` (vec4 at 48) holds primitive-specific data; interpretation
///   depends on `type_tag`. Closes the struct at 64 bytes (multiple of 16).
///
/// CSG blend metadata used to live here as `blend_mode` / `blend_smoothness`;
/// composition is now expressed by the token SSBO and lives in
/// [`super::csg_tree::Token`].
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub(super) struct SdfPrimitive {
    pub position: [f32; 3],
    pub type_tag: u32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub _pad0: f32,
    pub params: [f32; 4],
}

/// Matches `SceneMeta` in the WGSL shader.
///
/// `primitive_count` is the length of the primitives SSBO. `bvh_n` is
/// the number of leaves in the currently-bound BVH — i.e. the number
/// of primitives that participated in the last completed build. The
/// fragment shader treats `bvh_n == 0` as 'no scene' and returns the
/// sky background for every pixel; this is what we want both before
/// the first build resolves and when the scene is genuinely empty.
///
/// `has_intersects` / `has_subs` enable the optional branches of the
/// fixed default tree:
///   `smooth_subtract(smooth_intersect(adds, ints, k_int), subs, k_sub)`.
/// `k_int_scene` / `k_sub_scene` are the per-role smoothness maxima for
/// those final combination steps.
///
/// `skip_internal_sky = 1` tells the fragment shader to discard on miss
/// instead of drawing its internal vertical gradient. Set this when a
/// separate sky pass (e.g. `SkyRenderPass`) ran before us and already
/// filled the background — the ray-march pass then becomes additive on
/// top of that sky, only writing colors where rays actually hit SDFs.
///
/// Layout: 64 bytes total, std140-uniform clean (vec4 fields aligned to
/// 16 byte offsets). Verified by an `offset_of!` test.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(super) struct SceneMeta {
    pub primitive_count: u32,
    pub bvh_n: u32,
    pub skip_internal_sky: u32,
    pub has_intersects: u32,
    pub has_subs: u32,
    pub k_int_scene: f32,
    pub k_sub_scene: f32,
    pub _pad0: u32,
    pub sky_top: [f32; 4],
    pub sky_bottom: [f32; 4],
}

impl Default for SceneMeta {
    fn default() -> Self {
        Self {
            primitive_count: 0,
            bvh_n: 0,
            skip_internal_sky: 0,
            has_intersects: 0,
            has_subs: 0,
            k_int_scene: 0.0,
            k_sub_scene: 0.0,
            _pad0: 0,
            sky_top: [0.5, 0.7, 1.0, 1.0],
            sky_bottom: [0.1, 0.2, 0.4, 1.0],
        }
    }
}

/// Raymarch-only per-primitive metadata. Lives in a separate storage
/// buffer (`@group(1) @binding(5)` in `raymarch_main.wgsl`) so non-
/// raymarch consumers (physics broadphase, frustum culling) don't pay
/// for fields they never read.
///
/// **4 bytes, std430 stride 4** — single `f32` for now. Future fields
/// (per-leaf material override, debug colour, etc.) extend the struct
/// without touching the shared `ome_bvh::LeafAabb`.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub(super) struct RaymarchPayload {
    pub smoothness: f32,
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdf_primitive_layout_is_64_bytes() {
        assert_eq!(std::mem::size_of::<SdfPrimitive>(), 64);
        assert_eq!(std::mem::align_of::<SdfPrimitive>(), 4);
    }

    #[test]
    fn raymarch_payload_layout_is_4_bytes() {
        assert_eq!(std::mem::size_of::<RaymarchPayload>(), 4);
        assert_eq!(std::mem::align_of::<RaymarchPayload>(), 4);
    }

    #[test]
    fn scene_meta_layout_is_64_bytes() {
        // Uniform buffer needs std140-clean alignment. vec4 fields
        // (sky_top / sky_bottom) must land on 16-byte offsets.
        assert_eq!(std::mem::size_of::<SceneMeta>(), 64);
    }

    #[test]
    fn scene_meta_field_offsets_match_wgsl() {
        use std::mem::offset_of;
        assert_eq!(offset_of!(SceneMeta, primitive_count), 0);
        assert_eq!(offset_of!(SceneMeta, bvh_n), 4);
        assert_eq!(offset_of!(SceneMeta, skip_internal_sky), 8);
        assert_eq!(offset_of!(SceneMeta, has_intersects), 12);
        assert_eq!(offset_of!(SceneMeta, has_subs), 16);
        assert_eq!(offset_of!(SceneMeta, k_int_scene), 20);
        assert_eq!(offset_of!(SceneMeta, k_sub_scene), 24);
        // _pad0 lands at 28; sky_top must start at 32 to keep the vec4
        // 16-byte aligned.
        assert_eq!(offset_of!(SceneMeta, sky_top), 32);
        assert_eq!(offset_of!(SceneMeta, sky_bottom), 48);
    }
}
