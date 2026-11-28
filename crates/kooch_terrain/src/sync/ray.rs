//! Where a ray meets a planet's ground (#2002). The blocks are written by the GPU and have no CPU
//! triangles, so the ray is tested against the generator's CPU twin: the same SDF the blocks are
//! meshed from, exact rather than at the level a block happens to show.

use glam::{DVec3, Mat3, Vec3};
use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::hierarchy::WorldOrigin;

use super::{PlanetTerrain, gather};
use crate::eval::Generator;
use crate::lod::{Shape, voxel_size};
use crate::stamp::Stack;

/// Generator evaluations one planet may spend on a ray: a click or a drag, never a stall.
const EVALUATIONS: u32 = 2048;

/// Bisection steps once a segment shorter than a voxel brackets the surface.
const REFINE: u32 = 12;

/// A planet's ground struck by a ray.
#[derive(Clone, Copy, Debug)]
pub struct GroundHit {
    pub entity: Entity,
    /// Along the ray, in the ray's units.
    pub distance: f32,
    /// Unit, in the ray's space.
    pub normal: Vec3,
}

/// Every planet whose ground the ray from `start` along `direction` meets, one hit each, in no
/// order. Both are relative to the world origin, as a camera's `GlobalTransform` is.
pub fn ground_hits(resources: &Resources, start: Vec3, direction: Vec3) -> Vec<GroundHit> {
    let Some(state) = resources.get::<PlanetTerrain>() else {
        return Vec::new();
    };
    let origin = WorldOrigin::get(resources);
    let mut hits = Vec::new();
    for planet in gather(resources) {
        let Some(index) = state.roster.index_of(planet.entity) else {
            continue;
        };
        let i = index as usize;
        let (Some(Some(generator)), Some(shape)) =
            (state.generators.get(i), state.planets.shape.get(i))
        else {
            continue;
        };
        // In f64 from the planet's own cell: narrowed, the centre a radius away snaps by metres.
        let centre = planet.at - origin.from(Some(&planet.cell));
        let basis = Mat3::from_mat4(planet.world).as_dmat3();
        if !(basis.determinant().abs() > 1e-12) {
            continue;
        }
        let to_local = basis.inverse();
        let ray = Ray {
            from: to_local * (start.as_dvec3() - centre),
            along: to_local * direction.as_dvec3(),
        };
        let Some(stack) = state.stacks.get(i) else {
            continue;
        };
        let planet_ray = PlanetRay {
            generator,
            stack,
            shape,
            index,
        };
        let Some(t) = planet_ray.first(ray) else {
            continue;
        };
        let gradient = planet_ray.gradient(ray.at(t));
        // A normal is a covector: the inverse transpose carries it out of the planet's space.
        let normal = (to_local.transpose() * gradient).as_vec3();
        hits.push(GroundHit {
            entity: planet.entity,
            distance: t as f32,
            normal: normal.normalize_or(Vec3::Y),
        });
    }
    hits
}

#[derive(Clone, Copy)]
struct Ray {
    from: DVec3,
    along: DVec3,
}

impl Ray {
    fn at(self, t: f64) -> DVec3 {
        self.from + self.along * t
    }
}

struct PlanetRay<'a> {
    generator: &'a Generator,
    stack: &'a Stack,
    shape: &'a Shape,
    index: u32,
}

impl PlanetRay<'_> {
    /// The ground's SDF at `point`: the generator's, the stamps applied.
    fn sample(&self, point: DVec3) -> Option<f64> {
        let d = self.generator.sample(self.shape, point, self.index)?;
        Some(self.stack.apply(point, f64::from(d)))
    }

    /// The nearest `t` where the ray enters the ground. Segments the generator bounds above zero
    /// are skipped whole; the rest split, nearest half first, down to a voxel.
    fn first(&self, ray: Ray) -> Option<f64> {
        let (near, far) = self.span(ray)?;
        let voxel = voxel_size(self.shape, 0);
        let speed = ray.along.length();
        let mut stack = vec![(near, far)];
        let mut spent = 0;
        while let Some((a, b)) = stack.pop() {
            if spent >= EVALUATIONS {
                return None;
            }
            spent += 1;
            let (p, q) = (ray.at(a), ray.at(b));
            let (min, max) = (p.min(q), p.max(q));
            let range = self
                .generator
                .range(self.shape, min, max, self.index)
                .map(|range| self.stack.bound(min, max, range));
            if range.is_some_and(|(lo, _)| lo > 0.0) {
                continue;
            }
            if (b - a) * speed > voxel {
                let mid = (a + b) * 0.5;
                stack.push((mid, b));
                stack.push((a, mid));
                continue;
            }
            spent += 2;
            if let Some(t) = self.cross(ray, a, b) {
                return Some(t);
            }
        }
        None
    }

    /// The part of the ray inside the planet's highest relief, from no earlier than its start.
    fn span(&self, ray: Ray) -> Option<(f64, f64)> {
        let reach = crate::lod::bounding(self.shape) + voxel_size(self.shape, 0);
        let a = ray.along.length_squared();
        let b = ray.from.dot(ray.along);
        let c = ray.from.length_squared() - reach * reach;
        let disc = b * b - a * c;
        if !(disc >= 0.0) || !(a > 0.0) {
            return None;
        }
        let root = disc.sqrt();
        let (near, far) = ((-b - root) / a, (-b + root) / a);
        (far > 0.0).then_some((near.max(0.0), far))
    }

    /// Where the SDF changes sign from outside to inside between `a` and `b`, bisected; `a` itself
    /// when the ray starts underground.
    fn cross(&self, ray: Ray, a: f64, b: f64) -> Option<f64> {
        let sdf = |t: f64| self.sample(ray.at(t));
        let (mut lo, mut hi) = (a, b);
        if !(sdf(lo)? > 0.0) {
            return Some(lo);
        }
        if sdf(hi)? > 0.0 {
            return None;
        }
        for _ in 0..REFINE {
            let mid = (lo + hi) * 0.5;
            match sdf(mid)? > 0.0 {
                true => lo = mid,
                false => hi = mid,
            }
        }
        Some(hi)
    }

    /// The SDF's gradient at `point`, by central differences a voxel wide.
    fn gradient(&self, point: DVec3) -> DVec3 {
        let h = voxel_size(self.shape, 0);
        let sdf = |p: DVec3| self.sample(p).unwrap_or(0.0);
        DVec3::from_array(
            [DVec3::X, DVec3::Y, DVec3::Z].map(|e| sdf(point + e * h) - sdf(point - e * h)),
        )
    }
}
