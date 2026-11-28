//! Building a [`Field`] from measured values: an image's heights, or a mesh's distances.

use super::{FIELD_DISTANCE, FIELD_HEIGHT, Field, file::MOST_TEXELS};

impl Field {
    /// A heightmap of `width` × `depth` values, x fastest, rescaled so its lowest is 0 and its
    /// highest 1. `None` when the sizes disagree, it is too large, or holds a NaN.
    pub fn from_heights(width: u32, depth: u32, heights: &[f32]) -> Option<Self> {
        let (lo, hi) = range(heights)?;
        let span = (hi - lo).max(f32::MIN_POSITIVE);
        let unit: Vec<f32> = heights.iter().map(|h| (h - lo) / span).collect();
        build(FIELD_HEIGHT, [width, 1, depth], [1.0; 3], &unit, (0.0, 1.0))
    }

    /// [`Field::from_heights`] with at most `most` texels along the longer side: each texel the
    /// mean of the source texels it covers. Past a voxel's size, texels only cost memory.
    pub fn from_heights_at(width: u32, depth: u32, heights: &[f32], most: u32) -> Option<Self> {
        let longest = width.max(depth);
        if longest <= most.max(2) {
            return Self::from_heights(width, depth, heights);
        }
        if u64::from(width) * u64::from(depth) != heights.len() as u64 {
            return None;
        }
        let scale = f64::from(most.max(2)) / f64::from(longest);
        let shrink = |n: u32| ((f64::from(n) * scale).round() as u32).max(2);
        let (w, d) = (shrink(width), shrink(depth));
        let span = |i: u32, n: u32, of: u32| {
            let a = (u64::from(i) * u64::from(of) / u64::from(n)) as usize;
            let b = (u64::from(i + 1) * u64::from(of)).div_ceil(u64::from(n)) as usize;
            a..b.max(a + 1)
        };
        let mut out = Vec::with_capacity((w * d) as usize);
        for z in 0..d {
            for x in 0..w {
                let (mut sum, mut count) = (0.0f64, 0u32);
                for sz in span(z, d, depth) {
                    for sx in span(x, w, width) {
                        sum += f64::from(
                            heights
                                .get(sx + sz * width as usize)
                                .copied()
                                .unwrap_or(0.0),
                        );
                        count += 1;
                    }
                }
                out.push((sum / f64::from(count.max(1))) as f32);
            }
        }
        Self::from_heights(w, d, &out)
    }

    /// Signed distances over a `size` grid, in units of the box's largest half-extent, whose
    /// half-extents over that largest are `extent`.
    pub fn from_distances(size: [u32; 3], extent: [f32; 3], distances: &[f32]) -> Option<Self> {
        let bounds = range(distances)?;
        build(FIELD_DISTANCE, size, extent, distances, bounds)
    }
}

fn range(values: &[f32]) -> Option<(f32, f32)> {
    if values.is_empty() || values.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let lo = values.iter().copied().fold(f32::INFINITY, f32::min);
    let hi = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    Some((lo, hi))
}

fn build(
    kind: u32,
    size: [u32; 3],
    extent: [f32; 3],
    values: &[f32],
    (lo, hi): (f32, f32),
) -> Option<Field> {
    let count = size.iter().map(|&n| u64::from(n)).product::<u64>();
    if size.contains(&0) || count > MOST_TEXELS || count != values.len() as u64 {
        return None;
    }
    let span = (hi - lo).max(f32::MIN_POSITIVE);
    let texels: Vec<u16> = values
        .iter()
        .map(|v| (((v - lo) / span).clamp(0.0, 1.0) * 65535.0).round() as u16)
        .collect();
    let mut field = Field {
        kind,
        size,
        extent,
        lo,
        hi,
        slope: 0.0,
        texels: texels.into(),
    };
    field.slope = steepest(&field);
    Some(field)
}

/// The largest change between neighbouring texels, measured on what the GPU decodes.
fn steepest(field: &Field) -> f32 {
    let [nx, ny, nz] = field.size;
    let mut most = 0.0f32;
    for z in 0..nz {
        for y in 0..ny {
            for x in 0..nx {
                let here = field.at(x, y, z);
                for (dx, dy, dz) in [(1, 0, 0), (0, 1, 0), (0, 0, 1)] {
                    let (a, b, c) = (x + dx, y + dy, z + dz);
                    if a < nx && b < ny && c < nz {
                        most = most.max((field.at(a, b, c) - here).abs());
                    }
                }
            }
        }
    }
    most
}
