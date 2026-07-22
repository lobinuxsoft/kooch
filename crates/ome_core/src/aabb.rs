//! Axis-aligned bounding box primitive.
//!
//! Lives in `ome_core` so every consumer — world streaming, voxel
//! storage, physics — composes against the same type with no conversion
//! glue. It was in `ome_bvh` until that crate was removed; the type
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
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn unit_box() -> Aabb {
        Aabb::new(Vec3::ZERO, Vec3::splat(1.0))
    }

    #[test]
    fn center_and_extents() {
        let b = Aabb::new(Vec3::ZERO, Vec3::new(2.0, 4.0, 6.0));
        assert_eq!(b.center(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(b.extents(), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn from_centre_round_trip() {
        let b = Aabb::from_centre(Vec3::new(5.0, 0.0, -3.0), Vec3::splat(2.0));
        assert!((b.center() - Vec3::new(5.0, 0.0, -3.0)).length() < EPS);
        assert!((b.extents() - Vec3::splat(2.0)).length() < EPS);
    }

    #[test]
    fn distance_squared_inside_is_zero() {
        let b = unit_box();
        assert!(b.distance_squared(Vec3::splat(0.5)) < EPS);
    }

    #[test]
    fn distance_squared_outside_corner() {
        let b = unit_box();
        // Closest point on the box is (0,0,0); distance² = 9 + 16 + 0 = 25.
        let d2 = b.distance_squared(Vec3::new(-3.0, -4.0, 0.0));
        assert!((d2 - 25.0).abs() < EPS);
    }

    #[test]
    fn intersects_sphere_inside() {
        let b = unit_box();
        assert!(b.intersects_sphere(Vec3::splat(0.5), 0.0));
    }

    #[test]
    fn intersects_sphere_grazing() {
        let b = unit_box();
        // Sphere centred at (-1, 0, 0) with radius 1 just touches min.x = 0.
        assert!(b.intersects_sphere(Vec3::new(-1.0, 0.5, 0.5), 1.0));
        // Same centre, radius 0.999 → no hit.
        assert!(!b.intersects_sphere(Vec3::new(-1.0, 0.5, 0.5), 0.999));
    }

    #[test]
    fn intersects_aabb_overlap_and_disjoint() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
        let b_overlap = Aabb::new(Vec3::splat(0.5), Vec3::splat(1.5));
        let b_disjoint = Aabb::new(Vec3::splat(2.0), Vec3::splat(3.0));
        let b_touching = Aabb::new(Vec3::splat(1.0), Vec3::splat(2.0));
        assert!(a.intersects_aabb(&b_overlap));
        assert!(!a.intersects_aabb(&b_disjoint));
        // Boundary inclusive.
        assert!(a.intersects_aabb(&b_touching));
    }

    #[test]
    fn contains_point_boundary_inclusive() {
        let b = unit_box();
        assert!(b.contains_point(Vec3::ZERO));
        assert!(b.contains_point(Vec3::splat(1.0)));
        assert!(b.contains_point(Vec3::splat(0.5)));
        assert!(!b.contains_point(Vec3::splat(-0.001)));
        assert!(!b.contains_point(Vec3::splat(1.001)));
    }

    #[test]
    fn ray_intersect_hit_from_outside() {
        let b = unit_box();
        // Ray from (-1, 0.5, 0.5) towards +X — must hit at t = 1.
        let hit = b.ray_intersect(Vec3::new(-1.0, 0.5, 0.5), Vec3::X);
        let (t_near, t_far) = hit.expect("expected hit");
        assert!((t_near - 1.0).abs() < EPS);
        assert!((t_far - 2.0).abs() < EPS);
    }

    #[test]
    fn ray_intersect_miss() {
        let b = unit_box();
        // Ray parallel to +Y at x = 2 — outside the slab on x.
        assert!(b.ray_intersect(Vec3::new(2.0, -1.0, 0.5), Vec3::Y).is_none());
    }

    #[test]
    fn ray_intersect_origin_inside() {
        let b = unit_box();
        let hit = b.ray_intersect(Vec3::splat(0.5), Vec3::X);
        let (t_near, t_far) = hit.expect("expected hit from inside");
        // t_near is negative (the back wall is behind), t_far > 0.
        assert!(t_near < 0.0);
        assert!((t_far - 0.5).abs() < EPS);
    }

    #[test]
    fn expand_grows_box() {
        let mut b = Aabb::EMPTY;
        b.expand(Vec3::new(1.0, 2.0, 3.0));
        b.expand(Vec3::new(-1.0, 0.0, 4.0));
        assert_eq!(b.min, Vec3::new(-1.0, 0.0, 3.0));
        assert_eq!(b.max, Vec3::new(1.0, 2.0, 4.0));
    }

    #[test]
    fn empty_sentinel_is_empty() {
        assert!(Aabb::EMPTY.is_empty());
        assert!(Aabb::default().is_empty());
        assert!(!unit_box().is_empty());
    }

    #[test]
    fn union_covers_both() {
        let a = Aabb::new(Vec3::ZERO, Vec3::splat(1.0));
        let b = Aabb::new(Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 1.0, 1.0));
        let u = a.union(&b);
        assert_eq!(u.min, Vec3::ZERO);
        assert_eq!(u.max, Vec3::new(3.0, 1.0, 1.0));
    }
}
