//! Ray intersection against a triangle.
//!
//! Lives here because two crates that cannot depend on each other both need it: a block answers
//! picks from its own faces (`kooch_blockmesh`) and a rendered mesh from its meshlets
//! (`kooch_render`). [`Aabb::ray_intersect`](crate::aabb::Aabb::ray_intersect) is the box half of
//! the same job.

use glam::Vec3;

/// How close to an edge still counts as inside it. Without a tolerance a ray threading the seam
/// between two triangles hits neither, and a mesh picks up pinholes along every shared edge.
const EDGE: f32 = 1e-7;

/// Distance along `direction` at which the ray meets triangle `a`-`b`-`c`, or `None`.
///
/// Möller–Trumbore: no plane equation and no precomputed normal, so it costs one cross product
/// more than a plane test and needs nothing stored per triangle — which is what makes it usable
/// against a buffer the GPU owns the layout of.
///
/// 🔴 **Two-sided.** Culling back faces would hide a face from the author standing on its inside,
/// which is exactly where a hollow block puts them.
///
/// `direction` need not be normalised; `distance` comes back in whatever units it carries, so a
/// caller working in an object's local space can compare the result against world-space distances
/// only if the transform is a rigid one.
pub fn triangle(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let ab = b - a;
    let ac = c - a;
    let pvec = direction.cross(ac);
    let determinant = ab.dot(pvec);
    // Parallel to the triangle's plane: no crossing, or every point of
    // one. Both answer "not this triangle".
    if determinant.abs() < EDGE {
        return None;
    }

    let inverse = 1.0 / determinant;
    let tvec = origin - a;
    let u = tvec.dot(pvec) * inverse;
    if !(-EDGE..=1.0 + EDGE).contains(&u) {
        return None;
    }

    let qvec = tvec.cross(ab);
    let v = direction.dot(qvec) * inverse;
    if v < -EDGE || u + v > 1.0 + EDGE {
        return None;
    }

    let distance = ac.dot(qvec) * inverse;
    // Behind the eye. A click selects what is in front of it.
    match distance > EDGE {
        true => Some(distance),
        false => None,
    }
}
