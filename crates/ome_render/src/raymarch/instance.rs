//! GPU instance + scene metadata layouts for the unified ray-march pipeline.
//!
//! The byte layout of [`SdfInstance`] matches the WGSL `SdfInstance`
//! struct in `raymarch_main.wgsl` under std430 storage-buffer rules.

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
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug)]
pub struct RayMarchParams {
    pub max_steps: u32,
    pub max_distance: f32,
    pub surface_threshold: f32,
    pub _pad: f32,
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
            _pad: 0.0,
        }
    }
}

/// Per-entity unified SDF instance (80 bytes).
///
/// Field offsets match the WGSL struct byte-for-byte:
/// - `position` (vec3 at 0) + `type_tag` (u32 at 12) fill the first 16-byte slot.
/// - `rotation` (vec4 at 16) is naturally 16-aligned.
/// - `scale` (vec3 at 32) + `_pad0` (f32 at 44) fill the next 16-byte slot.
/// - `params` (vec4 at 48) holds primitive-specific data; interpretation
///   depends on `type_tag`.
/// - `blend_mode` (u32 at 64) + `blend_smoothness` (f32 at 68) + `_pad1`
///   (vec2 at 72) close the struct at 80 bytes, a multiple of 16.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Default)]
pub(super) struct SdfInstance {
    pub position: [f32; 3],
    pub type_tag: u32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub _pad0: f32,
    pub params: [f32; 4],
    pub blend_mode: u32,
    pub blend_smoothness: f32,
    pub _pad1: [u32; 2],
}

/// Matches `SceneMeta` in the WGSL shader.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(super) struct SceneMeta {
    pub instance_count: u32,
    pub _pad0: [u32; 3],
    pub sky_top: [f32; 4],
    pub sky_bottom: [f32; 4],
}

impl Default for SceneMeta {
    fn default() -> Self {
        Self {
            instance_count: 0,
            _pad0: [0; 3],
            sky_top: [0.5, 0.7, 1.0, 1.0],
            sky_bottom: [0.1, 0.2, 0.4, 1.0],
        }
    }
}

/// Initial capacity for the SDF instance storage buffer (grows on demand).
pub(super) const INITIAL_INSTANCE_CAPACITY: u64 = 256;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdf_instance_layout_is_80_bytes() {
        assert_eq!(std::mem::size_of::<SdfInstance>(), 80);
        assert_eq!(std::mem::align_of::<SdfInstance>(), 4);
    }
}
