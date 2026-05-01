//! Cascade descriptor — host-side mirror of the WGSL uniform that
//! drives the GDF populate compute pass. 32-byte std140 layout, must
//! stay byte-for-byte identical to the `CascadeDescriptor` struct
//! declared in `crates/ome_render/shaders/gdf_populate.wgsl`.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Voxel pitch of cascade 0. PR-3 hardcodes the cascade-0 LOD; the
/// per-cascade pitch table lands with PR-9 alongside the planet-scale
/// dispatch.
pub const CASCADE_0_VOXEL_SIZE: f32 = 0.25;

/// Voxel count along one axis of cascade 0 (= `64`). Constant for v1;
/// the populate dispatch uses `voxel_count_per_axis / WORKGROUP_XY` Z-slabs.
pub const CASCADE_0_VOXELS_PER_AXIS: u32 = 64;

/// World-space side length of cascade 0 in metres (16 m, by design).
pub const CASCADE_0_SIDE_METRES: f32 =
    CASCADE_0_VOXEL_SIZE * CASCADE_0_VOXELS_PER_AXIS as f32;

/// Workgroup XY dimension for the populate compute shader (`8`). 8×8×1
/// = 64 threads = single RDNA wavefront, Z-slabs externalised.
pub const POPULATE_WORKGROUP_XY: u32 = 8;

/// Cascade descriptor pushed to the populate compute shader once per
/// frame. Layout pinned at 32 bytes; `_pad` keeps the trailing 16-byte
/// std140 chunk explicit so the WGSL struct can mirror the field
/// list without surprise alignment rounding.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct CascadeDescriptor {
    /// World origin of the cascade voxel grid, snapped to multiples of
    /// `voxel_size` so the cascade's lattice is shift-invariant under
    /// fractional camera motion.
    pub world_origin: [f32; 3],
    /// Voxel pitch in metres.
    pub voxel_size: f32,
    /// Voxel count along one axis. PR-3 hardcodes 64.
    pub voxel_count_per_axis: u32,
    /// std140 padding so the struct rounds to a 16-byte boundary
    /// without leaving an implicit gap.
    pub _pad: [u32; 3],
}

impl CascadeDescriptor {
    /// Cascade 0 default — 16 m side, voxel_size 0.25 m, 64³ voxels.
    pub fn cascade_0(world_origin: Vec3) -> Self {
        Self {
            world_origin: world_origin.to_array(),
            voxel_size: CASCADE_0_VOXEL_SIZE,
            voxel_count_per_axis: CASCADE_0_VOXELS_PER_AXIS,
            _pad: [0; 3],
        }
    }
}

/// Snap an arbitrary world-space point to the closest multiple of
/// `voxel_size` along each axis (round-toward-negative-infinity, the
/// same `floor` semantics WGSL uses). Idempotent under repeated
/// application — `snap(snap(p)) == snap(p)`.
pub fn snap_to_voxel_grid(p: Vec3, voxel_size: f32) -> Vec3 {
    Vec3::new(
        (p.x / voxel_size).floor() * voxel_size,
        (p.y / voxel_size).floor() * voxel_size,
        (p.z / voxel_size).floor() * voxel_size,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn cascade_descriptor_layout() {
        // Total 32 B = 2× std140 chunks. Asserting the layout pins the
        // host/WGSL contract — every field offset checked below.
        assert_eq!(size_of::<CascadeDescriptor>(), 32);
        assert_eq!(align_of::<CascadeDescriptor>(), 4);

        assert_eq!(offset_of!(CascadeDescriptor, world_origin), 0);
        assert_eq!(offset_of!(CascadeDescriptor, voxel_size), 12);
        assert_eq!(offset_of!(CascadeDescriptor, voxel_count_per_axis), 16);
        assert_eq!(offset_of!(CascadeDescriptor, _pad), 20);
    }

    #[test]
    fn cascade_0_constants_consistent() {
        // 16 m side = 64 voxels × 0.25 m.
        assert_eq!(
            CASCADE_0_VOXEL_SIZE * CASCADE_0_VOXELS_PER_AXIS as f32,
            CASCADE_0_SIDE_METRES
        );
        assert_eq!(CASCADE_0_SIDE_METRES, 16.0);

        // Dispatch math: 64 voxels / 8 workgroup-XY = 8 workgroups per
        // axis. Pin the assumption so a future cascade-size bump can't
        // silently break the dispatch.
        assert_eq!(CASCADE_0_VOXELS_PER_AXIS % POPULATE_WORKGROUP_XY, 0);
    }

    #[test]
    fn world_origin_snapping_is_idempotent() {
        let voxel = CASCADE_0_VOXEL_SIZE;
        for p in [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.1234, -7.7, 42.5),
            Vec3::new(-1.0, 0.0, 1.0),
            Vec3::new(1.625, -0.0001, 1000.7),
        ] {
            let s1 = snap_to_voxel_grid(p, voxel);
            let s2 = snap_to_voxel_grid(s1, voxel);
            assert_eq!(s1, s2, "snap not idempotent at {p:?}: {s1:?} -> {s2:?}");
            // Each component is exactly a voxel-grid multiple.
            assert_eq!((s1.x / voxel).fract(), 0.0);
            assert_eq!((s1.y / voxel).fract(), 0.0);
            assert_eq!((s1.z / voxel).fract(), 0.0);
        }
    }

    #[test]
    fn snap_floors_toward_negative_infinity() {
        let v = CASCADE_0_VOXEL_SIZE;
        // -0.1 m falls into the cell starting at -0.25 m, not 0.0 m —
        // mirrors the WGSL `floor(p / voxel_size) * voxel_size` the
        // compute shader can later be replaced with.
        let snapped = snap_to_voxel_grid(Vec3::new(-0.1, -0.1, -0.1), v);
        assert_eq!(snapped, Vec3::new(-0.25, -0.25, -0.25));
    }
}
