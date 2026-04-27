//! 30-bit 3D Morton (Z-order) codes — 10 bits per axis.
//!
//! Used as the spatial sort key when building the LBVH: leaves whose
//! Morton codes are close in numeric value live close in 3D space, so
//! sorting by Morton groups spatial neighbours into contiguous ranges
//! of the leaves array. Karras-style top-down LBVH construction
//! (PR-1 subtask 3) walks those ranges in O(N).
//!
//! Resolution is 1024 cells per axis (2¹⁰), enough for 1 B leaves at
//! sub-cell distinguishability. Coarser than the 21-bit-per-axis
//! variant (which uses `u64`), but stays in `u32` so the GPU port
//! (PR-3) can sort 32-bit keys with cheaper radix passes. The current
//! consumer (chunk activation) operates on at most a few thousand
//! chunks; the cell granularity is not the bottleneck.

use glam::Vec3;

/// 30-bit Morton code packed in the low bits of a `u32`.
///
/// Bit layout: `0 0 z9 y9 x9 z8 y8 x8 z7 y7 x7 ... z0 y0 x0`. The
/// upper 2 bits are always zero, leaving room for a 32-bit radix-sort
/// key whose ordering is stable across CPU and GPU implementations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MortonCode(pub u32);

impl MortonCode {
    pub const ZERO: Self = Self(0);

    /// Build from explicit per-axis cell indices. Inputs are masked to
    /// 10 bits each, so callers don't need to clamp.
    pub fn from_xyz(x: u32, y: u32, z: u32) -> Self {
        Self(expand_bits_10(x) | (expand_bits_10(y) << 1) | (expand_bits_10(z) << 2))
    }

    /// Build from normalised coordinates in `[0, 1]³`. Coordinates are
    /// clamped to the valid cell range — out-of-range inputs land at
    /// the nearest face cell instead of wrapping or panicking.
    pub fn from_normalized(p: Vec3) -> Self {
        let scaled = (p * 1024.0).clamp(Vec3::ZERO, Vec3::splat(1023.0));
        Self::from_xyz(scaled.x as u32, scaled.y as u32, scaled.z as u32)
    }

    /// Decode back into the per-axis cell indices.
    pub fn decode(self) -> (u32, u32, u32) {
        (
            compact_bits_10(self.0),
            compact_bits_10(self.0 >> 1),
            compact_bits_10(self.0 >> 2),
        )
    }
}

/// Insert two zero bits between each of the input's low 10 bits.
///
/// `0b...00 abcdefghij` → `0b00 a00 b00 c00 d00 e00 f00 g00 h00 i00 j`.
/// Standard Sean Eron Anderson bit-twiddling sequence; the magic
/// constants alternate between "compact" and "spread" patterns.
fn expand_bits_10(v: u32) -> u32 {
    let v = v & 0x3FF;
    let v = (v | (v << 16)) & 0x030000FF;
    let v = (v | (v << 8)) & 0x0300F00F;
    let v = (v | (v << 4)) & 0x030C30C3;
    let v = (v | (v << 2)) & 0x09249249;
    v
}

/// Inverse of [`expand_bits_10`]: collapse every third bit back into
/// the low 10 bits.
fn compact_bits_10(v: u32) -> u32 {
    let v = v & 0x09249249;
    let v = (v | (v >> 2)) & 0x030C30C3;
    let v = (v | (v >> 4)) & 0x0300F00F;
    let v = (v | (v >> 8)) & 0x030000FF;
    let v = (v | (v >> 16)) & 0x000003FF;
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_round_trip() {
        let m = MortonCode::from_xyz(0, 0, 0);
        assert_eq!(m, MortonCode::ZERO);
        assert_eq!(m.decode(), (0, 0, 0));
    }

    #[test]
    fn unit_axes_have_canonical_values() {
        // Single-bit inputs should produce single-bit outputs at the
        // matching axis position.
        assert_eq!(MortonCode::from_xyz(1, 0, 0).0, 0b001);
        assert_eq!(MortonCode::from_xyz(0, 1, 0).0, 0b010);
        assert_eq!(MortonCode::from_xyz(0, 0, 1).0, 0b100);
        assert_eq!(MortonCode::from_xyz(1, 1, 1).0, 0b111);
    }

    #[test]
    fn maximum_value_round_trip() {
        let m = MortonCode::from_xyz(1023, 1023, 1023);
        assert_eq!(m.decode(), (1023, 1023, 1023));
        // All 30 low bits set, top 2 bits zero.
        assert_eq!(m.0, 0x3FFFFFFF);
    }

    #[test]
    fn input_masked_to_10_bits() {
        // Values past 1023 are silently clamped via the bit mask.
        let m = MortonCode::from_xyz(2048 + 5, 0, 0);
        assert_eq!(m.decode(), (5, 0, 0));
    }

    #[test]
    fn round_trip_random_grid() {
        // Iterate a coarse grid and verify decode(encode(p)) == p.
        for x in (0..1024).step_by(63) {
            for y in (0..1024).step_by(127) {
                for z in (0..1024).step_by(251) {
                    let m = MortonCode::from_xyz(x, y, z);
                    assert_eq!(m.decode(), (x, y, z));
                }
            }
        }
    }

    #[test]
    fn from_normalized_clamps() {
        let lo = MortonCode::from_normalized(Vec3::new(-1.0, -1.0, -1.0));
        assert_eq!(lo.decode(), (0, 0, 0));
        let hi = MortonCode::from_normalized(Vec3::new(2.0, 2.0, 2.0));
        assert_eq!(hi.decode(), (1023, 1023, 1023));
    }

    #[test]
    fn from_normalized_centre() {
        let c = MortonCode::from_normalized(Vec3::splat(0.5));
        let (x, y, z) = c.decode();
        // 0.5 * 1024 = 512.
        assert_eq!((x, y, z), (512, 512, 512));
    }

    #[test]
    fn ordering_preserves_locality_within_octant() {
        // Two points in the same upper-octant cube produce codes that
        // share the high bit on each axis — close points → close codes.
        let near_a = MortonCode::from_xyz(700, 700, 700);
        let near_b = MortonCode::from_xyz(701, 700, 700);
        let far = MortonCode::from_xyz(50, 50, 50);
        // |a - b| should be much smaller than |a - far|.
        let diff_near = near_a.0.abs_diff(near_b.0);
        let diff_far = near_a.0.abs_diff(far.0);
        assert!(
            diff_near < diff_far,
            "expected locality: near diff {diff_near} < far diff {diff_far}"
        );
    }

    #[test]
    fn ordering_is_total() {
        // The derived `Ord` lets us sort a list of codes.
        let mut codes = vec![
            MortonCode::from_xyz(5, 0, 0),
            MortonCode::from_xyz(1, 0, 0),
            MortonCode::from_xyz(3, 0, 0),
        ];
        codes.sort();
        let xs: Vec<u32> = codes.iter().map(|c| c.decode().0).collect();
        assert_eq!(xs, vec![1, 3, 5]);
    }
}
