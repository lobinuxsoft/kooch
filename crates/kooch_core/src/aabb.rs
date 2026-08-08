//! Axis-aligned bounding box primitive.
//!
//! Lives in `kooch_core` so every consumer — world streaming, voxel
//! storage, physics — composes against the same type with no conversion
//! glue. It was in the BVH crate until that crate was removed; the type
//! itself is plain geometry and never belonged to the acceleration
//! structure that happened to host it.

use glam::Vec3;

/// Axis-aligned bounding box in f32. The simulation frame is
/// camera-relative once the hierarchical-coords system is wired in
/// (issue #50, merged via PR #314), so f32 has full precision near the
/// active origin — far chunks are unloaded before they leave it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// AABB inverted into "empty" sentinel: `min = +∞`, `max = -∞`.
    /// Any [`Self::expand`] with a real point yields a valid box.
    pub const EMPTY: Self = Self {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };

    pub const fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// Build from a centre + half-extents.
    pub fn from_centre(centre: Vec3, half: Vec3) -> Self {
        Self {
            min: centre - half,
            max: centre + half,
        }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn extents(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Returns `true` when this AABB has no positive volume on any
    /// axis (e.g. it's the [`Self::EMPTY`] sentinel, or a degenerate
    /// inverted box).
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    /// Squared distance from `point` to the closest boundary of the box.
    /// Returns `0.0` for points inside.
    pub fn distance_squared(&self, point: Vec3) -> f32 {
        let clamped = point.clamp(self.min, self.max);
        (point - clamped).length_squared()
    }

    /// Returns `true` when this AABB intersects a sphere `(centre, radius)`.
    pub fn intersects_sphere(&self, centre: Vec3, radius: f32) -> bool {
        self.distance_squared(centre) <= radius * radius
    }

    /// Returns `true` when this AABB intersects another AABB.
    pub fn intersects_aabb(&self, other: &Self) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
            && self.min.z <= other.max.z
            && self.max.z >= other.min.z
    }

    /// Returns `true` when `point` is inside (boundary inclusive).
    pub fn contains_point(&self, point: Vec3) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
            && point.z >= self.min.z
            && point.z <= self.max.z
    }

    /// Slab-test ray intersection. Returns `Some((t_near, t_far))` when
    /// the ray hits, with `t_near <= t_far`. The caller decides whether
    /// `t_near < 0` (origin inside) is acceptable.
    pub fn ray_intersect(&self, origin: Vec3, dir: Vec3) -> Option<(f32, f32)> {
        // 1/dir per axis; ±∞ for axis-aligned rays — handled correctly
        // by IEEE-754 math (a min/max on +∞ leaves the other slab term
        // dominant).
        let inv = Vec3::ONE / dir;
        let t1 = (self.min - origin) * inv;
        let t2 = (self.max - origin) * inv;
        let tmin = t1.min(t2);
        let tmax = t1.max(t2);
        let t_near = tmin.x.max(tmin.y).max(tmin.z);
        let t_far = tmax.x.min(tmax.y).min(tmax.z);
        if t_far >= t_near.max(0.0) {
            Some((t_near, t_far))
        } else {
            None
        }
    }

    /// Expand this AABB to include `point`.
    pub fn expand(&mut self, point: Vec3) {
        self.min = self.min.min(point);
        self.max = self.max.max(point);
    }

    /// Returns the union of `self` and `other`.
    pub fn union(&self, other: &Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg(test)]
mod tests;
