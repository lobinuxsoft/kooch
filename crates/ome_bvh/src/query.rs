//! Stack-based traversal for the four canonical BVH queries.
//!
//! All four follow the same loop:
//! 1. Pop a node index from a fixed-size stack.
//! 2. Cull by testing the node's AABB against the query primitive.
//! 3. If leaf → visit each owned item; if internal → push both children
//!    (right at left + 1, by construction in [`super::bvh::Bvh::build`]).
//!
//! Stack size is fixed at [`MAX_STACK_DEPTH`] = 32; this covers any
//! BVH up to 2³² ≈ 4 B leaves built balanced. Sphere-tracing's 16-slot
//! stack (used in the SDF raymarch CSG tree, #307) is half this — the
//! query traversal is more conservative because the BVH is built by
//! Morton sort and may degenerate slightly under pathological inputs.
//!
//! Each public query has two flavours:
//!
//! - `for_each_*(...)` — calls a `FnMut(&T)` closure per hit. Zero
//!   allocation per query; the consumer owns the output buffer if any.
//! - `query_*(...)` — convenience wrapper that collects hits into a
//!   `Vec<T>` (requires `T: Copy`). For tests / one-shot tools.

use glam::Vec3;

use crate::aabb::Aabb;
use crate::bvh::Bvh;

/// Maximum traversal stack depth. Conservative — covers ~4 B leaves
/// in a balanced tree.
pub const MAX_STACK_DEPTH: usize = 32;

impl<T: Copy> Bvh<T> {
    /// Visit every leaf payload whose AABB intersects the sphere
    /// `(centre, radius)`. The closure is called once per matching
    /// item; node pruning stops the traversal early on non-overlapping
    /// subtrees.
    pub fn for_each_sphere(&self, centre: Vec3, radius: f32, mut visit: impl FnMut(&T)) {
        if self.is_empty() {
            return;
        }
        let radius_sq = radius * radius;
        traverse(self, |aabb| aabb.distance_squared(centre) <= radius_sq, &mut visit);
    }

    /// Visit every leaf payload whose AABB overlaps `query`.
    pub fn for_each_aabb(&self, query: Aabb, mut visit: impl FnMut(&T)) {
        if self.is_empty() {
            return;
        }
        traverse(self, |aabb| aabb.intersects_aabb(&query), &mut visit);
    }

    /// Visit every leaf payload whose AABB contains `point`.
    pub fn for_each_point(&self, point: Vec3, mut visit: impl FnMut(&T)) {
        if self.is_empty() {
            return;
        }
        traverse(self, |aabb| aabb.contains_point(point), &mut visit);
    }

    /// Visit every leaf payload whose AABB is hit by the ray
    /// `(origin, dir)` within `t_max`. `dir` does not need to be
    /// normalised — the slab test is scale-invariant — but `t_max`
    /// is measured in units of `dir`.
    pub fn for_each_ray(
        &self,
        origin: Vec3,
        dir: Vec3,
        t_max: f32,
        mut visit: impl FnMut(&T),
    ) {
        if self.is_empty() {
            return;
        }
        traverse(
            self,
            |aabb| match aabb.ray_intersect(origin, dir) {
                Some((t_near, t_far)) => t_far >= 0.0 && t_near <= t_max,
                None => false,
            },
            &mut visit,
        );
    }

    /// Collect all sphere-overlapping payloads into a `Vec`.
    pub fn query_sphere(&self, centre: Vec3, radius: f32) -> Vec<T> {
        let mut out = Vec::new();
        self.for_each_sphere(centre, radius, |t| out.push(*t));
        out
    }

    /// Collect all AABB-overlapping payloads into a `Vec`.
    pub fn query_aabb(&self, query: Aabb) -> Vec<T> {
        let mut out = Vec::new();
        self.for_each_aabb(query, |t| out.push(*t));
        out
    }

    /// Collect all payloads whose AABB contains `point`.
    pub fn query_point(&self, point: Vec3) -> Vec<T> {
        let mut out = Vec::new();
        self.for_each_point(point, |t| out.push(*t));
        out
    }

    /// Collect all payloads whose AABB is hit by the ray.
    pub fn query_ray(&self, origin: Vec3, dir: Vec3, t_max: f32) -> Vec<T> {
        let mut out = Vec::new();
        self.for_each_ray(origin, dir, t_max, |t| out.push(*t));
        out
    }
}

/// Generic stack-based traversal. `cull` returns `true` when the
/// node's AABB is potentially hit by the query (subtree continues),
/// `false` when the entire subtree is pruned. `visit` receives one
/// reference per leaf payload.
fn traverse<T: Copy>(
    bvh: &Bvh<T>,
    mut cull: impl FnMut(&Aabb) -> bool,
    visit: &mut impl FnMut(&T),
) {
    let mut stack: [u32; MAX_STACK_DEPTH] = [0; MAX_STACK_DEPTH];
    let mut sp: usize = 0;
    stack[sp] = 0;
    sp += 1;

    while sp > 0 {
        sp -= 1;
        let idx = stack[sp];
        let node = &bvh.nodes[idx as usize];

        let aabb = Aabb::new(Vec3::from(node.aabb_min), Vec3::from(node.aabb_max));
        if !cull(&aabb) {
            continue;
        }

        if node.is_leaf() {
            let first = node.left_or_first as usize;
            let count = node.count as usize;
            for i in 0..count {
                visit(&bvh.leaves[first + i]);
            }
        } else {
            // Right child at left + 1, by `Bvh::build` invariant.
            debug_assert!(
                sp + 2 <= MAX_STACK_DEPTH,
                "BVH traversal stack overflow at depth {sp}; tree is degenerate"
            );
            stack[sp] = node.left_or_first;
            sp += 1;
            stack[sp] = node.left_or_first + 1;
            sp += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn aabb_at(centre: Vec3, half: f32) -> Aabb {
        Aabb::from_centre(centre, Vec3::splat(half))
    }

    #[test]
    fn empty_bvh_no_hits() {
        let bvh: Bvh<u32> = Bvh::empty();
        assert_eq!(bvh.query_sphere(Vec3::ZERO, 100.0), Vec::<u32>::new());
        assert_eq!(bvh.query_aabb(aabb_at(Vec3::ZERO, 100.0)), Vec::<u32>::new());
        assert_eq!(bvh.query_point(Vec3::ZERO), Vec::<u32>::new());
        assert_eq!(bvh.query_ray(Vec3::ZERO, Vec3::X, 100.0), Vec::<u32>::new());
    }

    #[test]
    fn sphere_full_overlap_returns_all() {
        let items: Vec<(u32, Aabb)> = (0..8u32)
            .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
            .collect();
        let bvh = Bvh::build(items);
        // Sphere at 0 with radius huge — covers all.
        let mut hits = bvh.query_sphere(Vec3::ZERO, 1000.0);
        hits.sort();
        assert_eq!(hits, (0..8u32).collect::<Vec<_>>());
    }

    #[test]
    fn sphere_no_overlap_returns_empty() {
        let items: Vec<(u32, Aabb)> = (0..8u32)
            .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
            .collect();
        let bvh = Bvh::build(items);
        // Sphere far away.
        assert!(bvh.query_sphere(Vec3::new(1000.0, 0.0, 0.0), 1.0).is_empty());
    }

    #[test]
    fn sphere_partial_overlap_brute_force_match() {
        // 100 items in a unit cube, sphere at centre with various radii.
        let mut items = Vec::new();
        let mut rng_state = 0x1234567u32;
        let mut rand = || {
            rng_state = rng_state.wrapping_mul(1103515245).wrapping_add(12345);
            (rng_state >> 16) as f32 / 32768.0
        };
        for i in 0..100u32 {
            let p = Vec3::new(rand(), rand(), rand()) * 10.0;
            items.push((i, aabb_at(p, 0.2)));
        }
        let bvh = Bvh::build(items.clone());

        for radius in [0.5, 1.0, 3.0, 10.0] {
            let centre = Vec3::splat(5.0);
            let mut bvh_hits: HashSet<u32> = bvh.query_sphere(centre, radius).into_iter().collect();
            let brute: HashSet<u32> = items
                .iter()
                .filter(|(_, a)| a.intersects_sphere(centre, radius))
                .map(|(i, _)| *i)
                .collect();
            assert_eq!(bvh_hits, brute, "mismatch at radius {radius}");
            // Drain to silence unused.
            bvh_hits.clear();
        }
    }

    #[test]
    fn aabb_query_brute_force_match() {
        let mut items = Vec::new();
        for i in 0..50u32 {
            let x = (i as f32) * 0.5;
            items.push((i, aabb_at(Vec3::new(x, 0.0, 0.0), 0.2)));
        }
        let bvh = Bvh::build(items.clone());

        let q = Aabb::new(Vec3::new(5.0, -1.0, -1.0), Vec3::new(15.0, 1.0, 1.0));
        let bvh_hits: HashSet<u32> = bvh.query_aabb(q).into_iter().collect();
        let brute: HashSet<u32> = items
            .iter()
            .filter(|(_, a)| a.intersects_aabb(&q))
            .map(|(i, _)| *i)
            .collect();
        assert_eq!(bvh_hits, brute);
    }

    #[test]
    fn point_query_finds_containing_leaf() {
        let items = vec![
            (1u32, Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(1.0, 1.0, 1.0))),
            (2u32, Aabb::new(Vec3::new(2.0, 0.0, 0.0), Vec3::new(3.0, 1.0, 1.0))),
            (3u32, Aabb::new(Vec3::new(4.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 1.0))),
        ];
        let bvh = Bvh::build(items);
        assert_eq!(bvh.query_point(Vec3::new(0.5, 0.5, 0.5)), vec![1]);
        assert_eq!(bvh.query_point(Vec3::new(2.5, 0.5, 0.5)), vec![2]);
        assert_eq!(bvh.query_point(Vec3::new(4.5, 0.5, 0.5)), vec![3]);
        // Between boxes.
        assert!(bvh.query_point(Vec3::new(1.5, 0.5, 0.5)).is_empty());
    }

    #[test]
    fn ray_query_hit_through_aligned_boxes() {
        // Three boxes along +X. A ray from -1 along +X must hit all 3.
        let items = vec![
            (1u32, Aabb::new(Vec3::new(0.0, -0.5, -0.5), Vec3::new(1.0, 0.5, 0.5))),
            (2u32, Aabb::new(Vec3::new(2.0, -0.5, -0.5), Vec3::new(3.0, 0.5, 0.5))),
            (3u32, Aabb::new(Vec3::new(4.0, -0.5, -0.5), Vec3::new(5.0, 0.5, 0.5))),
        ];
        let bvh = Bvh::build(items);
        let hits: HashSet<u32> = bvh
            .query_ray(Vec3::new(-1.0, 0.0, 0.0), Vec3::X, 100.0)
            .into_iter()
            .collect();
        assert_eq!(hits, [1u32, 2, 3].into_iter().collect());
    }

    #[test]
    fn ray_query_t_max_clamps_far_hits() {
        let items = vec![
            (1u32, Aabb::new(Vec3::new(0.0, -0.5, -0.5), Vec3::new(1.0, 0.5, 0.5))),
            (2u32, Aabb::new(Vec3::new(10.0, -0.5, -0.5), Vec3::new(11.0, 0.5, 0.5))),
        ];
        let bvh = Bvh::build(items);
        // Ray from origin along +X, t_max = 5 → only first box.
        let hits: HashSet<u32> = bvh
            .query_ray(Vec3::ZERO, Vec3::X, 5.0)
            .into_iter()
            .collect();
        assert_eq!(hits, [1u32].into_iter().collect());
    }

    #[test]
    fn ray_query_miss() {
        let items = vec![
            (1u32, Aabb::new(Vec3::new(0.0, -0.5, -0.5), Vec3::new(1.0, 0.5, 0.5))),
        ];
        let bvh = Bvh::build(items);
        // Ray parallel to +Y, x = 5 — completely off.
        let hits = bvh.query_ray(Vec3::new(5.0, -10.0, 0.0), Vec3::Y, 100.0);
        assert!(hits.is_empty());
    }

    #[test]
    fn for_each_zero_alloc_callback() {
        let items: Vec<(u32, Aabb)> = (0..16u32)
            .map(|i| (i, aabb_at(Vec3::new(i as f32, 0.0, 0.0), 0.4)))
            .collect();
        let bvh = Bvh::build(items);
        let mut count = 0u32;
        bvh.for_each_sphere(Vec3::ZERO, 1000.0, |_| count += 1);
        assert_eq!(count, 16);
    }
}
