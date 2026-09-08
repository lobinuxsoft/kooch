//! Which face a ray hits.

use glam::Vec3;

use crate::BlockMesh;

/// A face the ray struck, and how far along it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    pub face: u32,
    /// Distance along the ray, in the units `direction` is measured in.
    pub distance: f32,
}

/// The nearest face `origin + t * direction` strikes, in the mesh's own
/// space.
///
/// Local, not world: the caller owns the entity's transform and can
/// invert it once, rather than this transforming every corner of every
/// face on every mouse move.
///
/// `direction` need not be normalised — `distance` comes back in
/// whatever units it carries, so an unnormalised ray gives a `t` in
/// units of its own length, which is what a caller comparing against
/// another `t` on the same ray wants.
///
/// Faces are convex, so a fan covers each exactly. Nearest hit wins,
/// including faces pointing away: a click inside an open mesh should
/// select the wall you can see through the hole, and a block being
/// edited is routinely seen from inside.
pub fn face_at(mesh: &BlockMesh, origin: Vec3, direction: Vec3) -> Option<Hit> {
    let mut nearest: Option<Hit> = None;

    for face in 0..mesh.face_count() {
        let Some(corners) = mesh.face(face) else {
            continue;
        };
        let anchor = mesh.positions()[corners[0] as usize];
        for corner in 1..corners.len() - 1 {
            let b = mesh.positions()[corners[corner] as usize];
            let c = mesh.positions()[corners[corner + 1] as usize];
            let Some(distance) = triangle_at(origin, direction, anchor, b, c) else {
                continue;
            };
            if nearest.is_none_or(|hit| distance < hit.distance) {
                nearest = Some(Hit {
                    face: face as u32,
                    distance,
                });
            }
        }
    }

    nearest
}

/// Distance along the ray to a triangle, or `None` when it misses.
///
/// Möller–Trumbore, two-sided. The determinant's sign says which way the
/// triangle faces and is deliberately not read: culling backfaces here
/// would make a face unselectable from the side an author is standing on
/// half the time.
fn triangle_at(origin: Vec3, direction: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    const EDGE: f32 = 1e-7;

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

#[cfg(test)]
mod tests;
