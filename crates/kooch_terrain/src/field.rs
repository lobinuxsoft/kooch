//! Shapes that come from data (#709): a heightmap, or a 3D grid of signed distances baked from a
//! mesh. A stamp stretches one over its box. The texels stay on the CPU too, and both sides filter
//! them by hand with the same weights, so collision follows what is drawn.

mod bake;
mod file;
#[cfg(test)]
mod tests;

use std::sync::Arc;

use glam::DVec3;

pub use file::{FIELD_EXTENSION, FieldLoader};

/// A height per texel over x and z, 0 at the box's floor and 1 at its top.
pub const FIELD_HEIGHT: u32 = 0;
/// A signed distance per texel, in units of the source box's largest half-extent.
pub const FIELD_DISTANCE: u32 = 1;

/// A `.kfield`: texels as `u16`, mapped onto `lo..hi`. x runs fastest, then y, then z; a height
/// field has one row in y.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub kind: u32,
    pub size: [u32; 3],
    /// The source's half-extents over the largest, so a baked mesh keeps its proportions.
    pub extent: [f32; 3],
    pub lo: f32,
    pub hi: f32,
    /// The largest change between neighbouring texels, decoded: what keeps the stamp a distance.
    pub slope: f32,
    pub texels: Arc<[u16]>,
}

impl Field {
    /// Whether both hold the very same texels: a field is cloned out of its asset every frame,
    /// and only a reload gives it new ones.
    pub(crate) fn same(&self, other: &Field) -> bool {
        Arc::ptr_eq(&self.texels, &other.texels)
    }

    pub(crate) fn decode(&self, texel: u16) -> f32 {
        self.lo + (self.hi - self.lo) * (f32::from(texel) / 65535.0)
    }

    fn at(&self, x: u32, y: u32, z: u32) -> f32 {
        let [nx, ny, _] = self.size;
        let index = x as usize + nx as usize * (y as usize + ny as usize * z as usize);
        self.decode(self.texels.get(index).copied().unwrap_or(0))
    }

    /// What one metre of the stamp's space is worth in the field's value, so the stamp stays a
    /// true distance however it is stretched: a steep map is flattened, a squashed volume shrunk.
    pub(crate) fn gain(&self, half: DVec3) -> f64 {
        let [nx, ny, nz] = self.size.map(|n| f64::from(n.max(2) - 1));
        let slope = f64::from(self.slope);
        match self.kind {
            FIELD_HEIGHT => {
                let step = (2.0 * half.x / nx)
                    .min(2.0 * half.z / nz)
                    .max(f64::MIN_POSITIVE);
                // Steepest along x and z at once, as a bilinear patch can be.
                let rise = slope * 2.0 * half.y / step;
                1.0 / (1.0 + 2.0 * rise * rise).sqrt()
            }
            _ => {
                let extent = DVec3::from_array(self.extent.map(f64::from)).max(DVec3::splat(1e-6));
                let metres = (half / extent).min_element();
                let step = (2.0 * extent / DVec3::new(nx, ny, nz)).min_element();
                let steepest = 3f64.sqrt() * slope / step.max(f64::MIN_POSITIVE);
                metres / steepest.max(1.0)
            }
        }
    }
}

/// The field's SDF at `p` in a stamp of half-extents `half`, `gain` from [`Field::gain`]. Mirrors
/// `stamp_field` in `generate.wgsl`.
pub(crate) fn distance(field: &Field, p: DVec3, half: DVec3, gain: f64) -> f64 {
    let half = half.max(DVec3::splat(1e-6));
    let q = p.clamp(-half, half);
    let t = (q / half) * 0.5 + 0.5;
    let [nx, ny, nz] = field.size.map(|n| n.max(1));
    let along = |t: f64, n: u32| -> (u32, u32, f64) {
        let x = t * f64::from(n - 1);
        let i = (x.floor() as u32).min(n.saturating_sub(2));
        let j = (i + 1).min(n - 1);
        (i, j, (x - f64::from(i)).clamp(0.0, 1.0))
    };
    let (x0, x1, fx) = along(t.x, nx);
    let (z0, z1, fz) = along(t.z, nz);
    let lerp = |a: f32, b: f32, f: f64| f64::from(a) + (f64::from(b) - f64::from(a)) * f;
    if field.kind == FIELD_HEIGHT {
        let row = |z| lerp(field.at(x0, 0, z), field.at(x1, 0, z), fx);
        let h = row(z0) + (row(z1) - row(z0)) * fz;
        let surface = -half.y + 2.0 * half.y * h;
        let b = p.abs() - half;
        let boxed = b.max(DVec3::ZERO).length() + b.max_element().min(0.0);
        return ((p.y - surface) * gain).max(boxed);
    }
    let (y0, y1, fy) = along(t.y, ny);
    let plane = |y| {
        let row = |z| lerp(field.at(x0, y, z), field.at(x1, y, z), fx);
        row(z0) + (row(z1) - row(z0)) * fz
    };
    let v = plane(y0) + (plane(y1) - plane(y0)) * fy;
    v * gain + (p - q).length()
}
