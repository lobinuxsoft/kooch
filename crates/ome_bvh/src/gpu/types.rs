//! GPU-side data layouts for the LBVH compute pipeline.
//!
//! Every struct here is `#[repr(C)]` + `Pod` + `Zeroable` and chosen to
//! match WGSL `std430` alignment without internal padding surprises.
//! The CPU builder (PR-1) and the GPU builder (this PR) share these
//! types — there is no separate "CPU bvh" / "GPU bvh" split.

use bytemuck::{Pod, Zeroable};

use crate::aabb::Aabb;

/// AABB packed for `std430` storage buffer use. 32 bytes, no internal
/// padding (each `vec3` is followed by an explicit `f32` padding slot
/// — required because `vec3<f32>` has 16-byte alignment in std430).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct GpuAabb {
    pub min: [f32; 3],
    pub _pad0: f32,
    pub max: [f32; 3],
    pub _pad1: f32,
}

impl From<Aabb> for GpuAabb {
    fn from(aabb: Aabb) -> Self {
        Self {
            min: aabb.min.into(),
            _pad0: 0.0,
            max: aabb.max.into(),
            _pad1: 0.0,
        }
    }
}

impl GpuAabb {
    pub fn to_aabb(self) -> Aabb {
        Aabb::new(self.min.into(), self.max.into())
    }
}

/// Scene-wide normalisation bounds + item count, uploaded as a single
/// uniform buffer for the Morton compute shader.
///
/// `inv_extent.x = 1.0 / (scene_max.x - scene_min.x)`. Zero on degenerate
/// (single-point) axes — the shader treats `0 * delta = 0` as "all items
/// land at cell 0" on that axis, which is the correct degenerate
/// behaviour (matches the CPU `Bvh::build`).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default, Debug)]
pub struct GpuSceneBounds {
    pub min: [f32; 3],
    pub _pad0: f32,
    pub inv_extent: [f32; 3],
    pub count: u32,
}

impl GpuSceneBounds {
    /// Build from a list of AABBs. Returns the uniform plus the
    /// `Aabb::EMPTY`-fallback bounds (`inv_extent = 0` on every axis)
    /// when the input is empty.
    pub fn from_aabbs(aabbs: &[Aabb]) -> Self {
        let scene = aabbs
            .iter()
            .fold(Aabb::EMPTY, |acc, a| acc.union(a));
        if aabbs.is_empty() || scene.is_empty() {
            return Self {
                min: [0.0; 3],
                _pad0: 0.0,
                inv_extent: [0.0; 3],
                count: aabbs.len() as u32,
            };
        }
        let extent = scene.max - scene.min;
        let inv = glam::Vec3::new(
            if extent.x > 0.0 { 1.0 / extent.x } else { 0.0 },
            if extent.y > 0.0 { 1.0 / extent.y } else { 0.0 },
            if extent.z > 0.0 { 1.0 / extent.z } else { 0.0 },
        );
        Self {
            min: scene.min.into(),
            _pad0: 0.0,
            inv_extent: inv.into(),
            count: aabbs.len() as u32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn gpu_aabb_is_32_bytes_aligned_4() {
        assert_eq!(std::mem::size_of::<GpuAabb>(), 32);
        assert_eq!(std::mem::align_of::<GpuAabb>(), 4);
    }

    #[test]
    fn gpu_aabb_round_trip() {
        let a = Aabb::new(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        let g: GpuAabb = a.into();
        assert_eq!(g.min, [1.0, 2.0, 3.0]);
        assert_eq!(g.max, [4.0, 5.0, 6.0]);
        assert_eq!(g.to_aabb(), a);
    }

    #[test]
    fn gpu_scene_bounds_is_32_bytes() {
        assert_eq!(std::mem::size_of::<GpuSceneBounds>(), 32);
    }

    #[test]
    fn scene_bounds_from_empty_input() {
        let s = GpuSceneBounds::from_aabbs(&[]);
        assert_eq!(s.count, 0);
        assert_eq!(s.inv_extent, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn scene_bounds_unions_aabbs() {
        let aabbs = [
            Aabb::new(Vec3::ZERO, Vec3::splat(1.0)),
            Aabb::new(Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 1.0, 1.0)),
        ];
        let s = GpuSceneBounds::from_aabbs(&aabbs);
        assert_eq!(s.count, 2);
        // Scene = [0,3] × [0,1] × [0,1], extent = (3,1,1), inv = (1/3,1,1).
        assert!((s.inv_extent[0] - 1.0 / 3.0).abs() < 1e-6);
        assert!((s.inv_extent[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn scene_bounds_degenerate_axis_inv_zero() {
        // Two AABBs that span x but coincide on y, z.
        let aabbs = [
            Aabb::new(Vec3::new(0.0, 5.0, 5.0), Vec3::new(1.0, 5.0, 5.0)),
            Aabb::new(Vec3::new(2.0, 5.0, 5.0), Vec3::new(3.0, 5.0, 5.0)),
        ];
        let s = GpuSceneBounds::from_aabbs(&aabbs);
        assert!(s.inv_extent[0] > 0.0); // x has extent
        assert_eq!(s.inv_extent[1], 0.0); // y degenerate
        assert_eq!(s.inv_extent[2], 0.0); // z degenerate
    }
}
