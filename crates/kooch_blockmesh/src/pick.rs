//! Which part of a block the cursor is over.
//!
//! Faces are picked with a ray, vertices and edges in screen space. Not
//! a style choice: a vertex has no area and an edge no width, so a ray
//! misses both every time. What "close enough" means for them is a
//! number of PIXELS, and pixels only exist after the projection.
//!
//! godot-ply (MIT) approximates it with a world-space radius that grows
//! as `sqrt(distance) / 32`. That is a curve fitted to the projection
//! rather than the projection, and it drifts with field of view.

use glam::{Mat4, Vec2, Vec3};

use crate::{Adjacency, BlockMesh};

/// An element the cursor found, and how far away it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// Index of a face, edge or vertex, depending on what was asked.
    pub element: u32,
    /// Distance along the ray, in the units `direction` is measured in.
    pub distance: f32,
}

/// Mesh space to viewport pixels.
#[derive(Debug, Clone, Copy)]
pub struct Screen {
    /// `view_proj * model` — local straight to clip, one matrix.
    pub clip: Mat4,
    /// Viewport size in physical pixels.
    pub size: Vec2,
}

impl Screen {
    /// Where a local point lands, and its view depth.
    ///
    /// `None` behind the eye, where the perspective divide flips the
    /// point to the opposite side of the screen and every distance
    /// measured from it is a lie.
    fn project(&self, point: Vec3) -> Option<(Vec2, f32)> {
        let clip = self.clip * point.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        let pixels = Vec2::new(
            (ndc.x * 0.5 + 0.5) * self.size.x,
            (0.5 - ndc.y * 0.5) * self.size.y,
        );
        Some((pixels, clip.w))
    }
}

/// Nearest to the cursor wins; a tie in pixels is broken by depth.
///
/// Both halves earn their place. Ranking by depth alone hands a click to
/// whatever is closest to the eye even when you aimed at something
/// twenty pixels away from it; ranking by pixels alone leaves a cube's
/// front and back corners — which project to the SAME pixel — decided
/// by iteration order.
const TIE: f32 = 1.0;

fn closer(candidate: (f32, f32), best: (f32, f32)) -> bool {
    let (pixels, depth) = candidate;
    let (best_pixels, best_depth) = best;
    if pixels < best_pixels - TIE {
        return true;
    }
    (pixels - best_pixels).abs() <= TIE && depth < best_depth
}

/// The vertex under the cursor, within `radius` pixels.
pub fn vertex_at(mesh: &BlockMesh, screen: Screen, cursor: Vec2, radius: f32) -> Option<Hit> {
    let mut best: Option<(u32, f32, f32)> = None;

    for (index, position) in mesh.positions().iter().enumerate() {
        let Some((pixels, depth)) = screen.project(*position) else {
            continue;
        };
        let distance = pixels.distance(cursor);
        if distance > radius {
            continue;
        }
        if best.is_none_or(|(_, held, held_depth)| closer((distance, depth), (held, held_depth))) {
            best = Some((index as u32, distance, depth));
        }
    }

    best.map(|(element, _, depth)| Hit {
        element,
        distance: depth,
    })
}

/// The edge under the cursor, within `radius` pixels.
///
/// An edge with an endpoint behind the eye is skipped rather than
/// clipped: the case needs a near-plane intersection to be right, and
/// getting it subtly wrong picks an edge that is nowhere near the
/// cursor. The vertices at its ends stay pickable.
pub fn edge_at(
    mesh: &BlockMesh,
    adjacency: &Adjacency,
    screen: Screen,
    cursor: Vec2,
    radius: f32,
) -> Option<Hit> {
    let mut best: Option<(u32, f32, f32)> = None;

    for edge in 0..adjacency.edge_count() as u32 {
        let Some([from, to]) = adjacency.edge_corners(edge) else {
            continue;
        };
        let (Some((a, a_depth)), Some((b, b_depth))) = (
            screen.project(mesh.positions()[from as usize]),
            screen.project(mesh.positions()[to as usize]),
        ) else {
            continue;
        };

        let (distance, along) = segment_at(cursor, a, b);
        if distance > radius {
            continue;
        }
        let depth = a_depth + (b_depth - a_depth) * along;
        if best.is_none_or(|(_, held, held_depth)| closer((distance, depth), (held, held_depth))) {
            best = Some((edge, distance, depth));
        }
    }

    best.map(|(element, _, depth)| Hit {
        element,
        distance: depth,
    })
}

/// Distance from a point to a segment, and how far along the segment the
/// nearest point sits.
fn segment_at(point: Vec2, from: Vec2, to: Vec2) -> (f32, f32) {
    let span = to - from;
    let length = span.length_squared();
    // A segment seen end-on is a point, and dividing by its length is
    // the NaN that would make every later comparison false.
    if length < 1e-12 {
        return (point.distance(from), 0.0);
    }
    let along = ((point - from).dot(span) / length).clamp(0.0, 1.0);
    (point.distance(from + span * along), along)
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
                    element: face as u32,
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
