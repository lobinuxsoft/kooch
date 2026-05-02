//! Cascade descriptor + multi-cascade GDF uniforms. Host-side mirror
//! of the WGSL types in `crates/ome_render/shaders/raymarch_gdf_sample.wgsl`.
//! Layouts are pinned by `cascade_descriptor_layout` /
//! `gdf_uniforms_layout` tests so any drift surfaces in CI before
//! reaching the GPU.
//!
//! PR-5 of epic #370 promotes the uniform from a single
//! `CascadeDescriptor` to `GdfUniforms { cascades: [CascadeDescriptor; 6] }`
//! (192 B). The fragment shader's `pick_cascade` walks finest →
//! coarsest and chooses the first cascade whose voxel pitch is at
//! least the per-step cone radius.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;

/// Number of GDF cascades (PR-5 of epic #370). 6 covers `~0.25 m`
/// near-field through `~512 km` far-field — see [`CASCADE_VOXEL_SIZES`].
pub const CASCADE_COUNT: usize = 6;

/// Per-cascade voxel pitch table. Geometric ratio `8` between
/// adjacent cascades — finest cascade picks up sub-metre detail at
/// the camera, coarsest covers the planetary horizon. The 8× ratio
/// is borrowed from UE5 Lumen's GDF cascade chain and matches the
/// engine's planet-scale handheld budget: cascade 5 reaches the
/// LOD-0 streaming radius without leaving holes between cones.
pub const CASCADE_VOXEL_SIZES: [f32; CASCADE_COUNT] =
    [0.25, 2.0, 16.0, 128.0, 1024.0, 8192.0];

/// Voxel count along one axis (every cascade is 64³). Constant across
/// cascades so the populate dispatch grid is the same on every level —
/// cascade `c`'s cube extent is `CASCADE_VOXEL_SIZES[c] * 64`.
pub const CASCADE_VOXELS_PER_AXIS: u32 = 64;

/// Cascade 0 voxel pitch — kept as a stable named alias for the PR-3
/// integration tests that pin numerics against this value.
pub const CASCADE_0_VOXEL_SIZE: f32 = CASCADE_VOXEL_SIZES[0];

/// Cascade 0 voxel count alias (re-exported as `CASCADE_0_VOXELS_PER_AXIS`
/// from `gdf/mod.rs`).
pub const CASCADE_0_VOXELS_PER_AXIS: u32 = CASCADE_VOXELS_PER_AXIS;

/// World-space side length of cascade 0 in metres (16 m, by design).
pub const CASCADE_0_SIDE_METRES: f32 =
    CASCADE_0_VOXEL_SIZE * CASCADE_0_VOXELS_PER_AXIS as f32;

/// Workgroup XY dimension for the populate compute shader (`8`). 8×8×1
/// = 64 threads = single RDNA wavefront, Z-slabs externalised.
pub const POPULATE_WORKGROUP_XY: u32 = 8;

/// World-space cube extent of cascade `c` in metres.
pub const fn cascade_cube_extent(c: usize) -> f32 {
    CASCADE_VOXEL_SIZES[c] * CASCADE_VOXELS_PER_AXIS as f32
}

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
        Self::for_cascade(0, world_origin)
    }

    /// Build a descriptor for cascade `c` at the supplied snapped
    /// world origin. Used by the per-cascade populate dispatch.
    pub fn for_cascade(c: usize, world_origin: Vec3) -> Self {
        Self {
            world_origin: world_origin.to_array(),
            voxel_size: CASCADE_VOXEL_SIZES[c],
            voxel_count_per_axis: CASCADE_VOXELS_PER_AXIS,
            _pad: [0; 3],
        }
    }
}

/// Multi-cascade fragment-shader uniform. Mirrors the WGSL
/// `GdfUniforms` declared in `raymarch_gdf_sample.wgsl`. 192 B
/// (`6 × CascadeDescriptor`) — already a multiple of the 16 B std140
/// alignment so no trailing pad is needed.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct GdfUniforms {
    pub cascades: [CascadeDescriptor; CASCADE_COUNT],
}

impl GdfUniforms {
    /// Default placement: every cascade rooted at `world_origin = 0`.
    /// `GdfState::dispatch_populate` overwrites a single descriptor
    /// each time it advances a cascade, so the fragment shader sees
    /// per-cascade origins independently.
    pub fn from_origins(origins: &[Vec3; CASCADE_COUNT]) -> Self {
        let mut cascades = [CascadeDescriptor::default(); CASCADE_COUNT];
        for (c, origin) in origins.iter().enumerate() {
            cascades[c] = CascadeDescriptor::for_cascade(c, *origin);
        }
        Self { cascades }
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
    fn gdf_uniforms_layout() {
        // 6 cascades × 32 B = 192 B; alignment 4 (Pod from `[CD; 6]`).
        // WGSL std140 will see this as `array<CascadeDescriptor, 6>`
        // with stride 32 (= 16-aligned, no padding round-up needed).
        assert_eq!(size_of::<GdfUniforms>(), 32 * CASCADE_COUNT);
        assert_eq!(align_of::<GdfUniforms>(), 4);
        assert_eq!(offset_of!(GdfUniforms, cascades), 0);
    }

    #[test]
    fn cascade_voxel_sizes_geometric_ratio() {
        // 8× ratio between adjacent cascades; deviating breaks
        // `pick_cascade`'s "first cascade with voxel_size >= cone_radius"
        // guarantee — non-monotonic voxel sizes would let coarser
        // cascades win for close rays. Pin the ratio.
        for c in 1..CASCADE_COUNT {
            let ratio = CASCADE_VOXEL_SIZES[c] / CASCADE_VOXEL_SIZES[c - 1];
            assert!(
                (ratio - 8.0).abs() < 1.0e-3,
                "cascade {c} voxel ratio {ratio} != 8.0"
            );
        }
    }

    #[test]
    fn cascade_cube_extents_cover_planet_scale_horizon() {
        // Cascade 5 must cover the LOD-0 streaming radius (~256 m for
        // chunks at level 0) by orders of magnitude — handheld view
        // distance peaks ~16 km, planet horizon ~~5000 km. 524 km of
        // cascade-5 reach covers both without falling back to the
        // coarsest-AABB sphere-trace floor.
        assert!(cascade_cube_extent(5) > 100_000.0);
        // Cascade 0 must keep voxel pitch sub-metre so the editor's
        // close-up gizmos render without aliasing.
        assert!(CASCADE_VOXEL_SIZES[0] < 1.0);
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
