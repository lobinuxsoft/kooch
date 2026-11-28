//! Signed distances from a triangle soup, for baking a mesh into a terrain field (#709).

use glam::Vec3;
use rapier3d::parry::query::PointQuery;
use rapier3d::parry::shape::TriMesh;
use rayon::prelude::*;

/// Each of `points`' distance to the mesh, negative inside. The size comes from parry's BVH; the
/// sign from the generalized winding number (Jacobson et al. 2013), which holds on a mesh that is
/// open or has loose parts, as Suzanne does. `None` when parry refuses the triangles.
pub fn signed_distances(
    vertices: &[Vec3],
    indices: &[[u32; 3]],
    points: &[Vec3],
) -> Option<Vec<f32>> {
    let mesh = TriMesh::new(vertices.to_vec(), indices.to_vec()).ok()?;
    let triangles: Vec<[Vec3; 3]> = indices
        .iter()
        .filter_map(|t| {
            Some([
                *vertices.get(t[0] as usize)?,
                *vertices.get(t[1] as usize)?,
                *vertices.get(t[2] as usize)?,
            ])
        })
        .collect();
    Some(
        points
            .par_iter()
            .map(|&p| {
                let size = (mesh.project_local_point(p, false).point - p).length();
                match winding(&triangles, p) > 0.5 {
                    true => -size,
                    false => size,
                }
            })
            .collect(),
    )
}

/// How many times the surface wraps `p`: 1 inside a closed mesh, 0 outside, between at a hole.
fn winding(triangles: &[[Vec3; 3]], p: Vec3) -> f32 {
    let mut total = 0.0f32;
    for [a, b, c] in triangles {
        let (a, b, c) = (*a - p, *b - p, *c - p);
        let (la, lb, lc) = (a.length(), b.length(), c.length());
        let numerator = a.dot(b.cross(c));
        let denominator = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
        total += 2.0 * numerator.atan2(denominator);
    }
    total / (4.0 * std::f32::consts::PI)
}
