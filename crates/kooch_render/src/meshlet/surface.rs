//! Where a ray meets a [`MeshletMesh`]'s actual surface (#1435).
//!
//! 🔴 A box is not a surface. A sphere's bounding box answers a ray a long way from the sphere, and
//! anything that lands something *on* geometry — snapping a drag, dropping a prop — lands it in mid
//! air. Picking can live with the box because selecting roughly the right object is still the right
//! object; placement cannot.
//!
//! ⚠️ **LOD 0 only.** The coarser levels of the chain are simplifications of the same surface, and
//! testing them too would return hits from a shape nobody is looking at.

use glam::Vec3;

use kooch_core::aabb::Aabb;

use super::asset::MeshletMesh;

/// Where a ray met the surface.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceHit {
    /// Distance along `direction`, in the units it carries.
    pub distance: f32,
    /// Geometric normal of the triangle struck, unnormalised length aside.
    pub normal: Vec3,
}

/// The nearest point at which `ray` enters the mesh's surface, in the mesh's own space.
///
/// Each meshlet carries its own AABB, which makes `meshlets` a ready-made one-level BVH: a ray that
/// misses a meshlet's box skips its whole triangle run. That is what keeps this affordable enough
/// to call once per frame during a drag — on a sphere a cursor ray crosses two or three meshlets
/// out of dozens.
pub fn ray_hit(mesh: &MeshletMesh, origin: Vec3, direction: Vec3) -> Option<SurfaceHit> {
    let mut nearest: Option<SurfaceHit> = None;

    for meshlet in mesh.meshlets.iter().filter(|m| m.lod_level == 0) {
        let bounds = Aabb::new(meshlet.aabb_min.into(), meshlet.aabb_max.into());
        // A box behind the eye is a box this ray never reaches. `far >= 0.0` rather than
        // `near >= 0.0` keeps the meshlet the eye is *inside*, whose far wall is still ahead.
        match bounds.ray_intersect(origin, direction) {
            Some((near, far)) if far >= 0.0 => {
                // Already have something closer than this box can possibly hold.
                if nearest.is_some_and(|hit| hit.distance < near) {
                    continue;
                }
            }
            _ => continue,
        }

        for triangle in 0..meshlet.triangle_count {
            let corner = |index: u32| -> Vec3 {
                // `meshlet_triangles` is one byte per corner, indexing the meshlet's own vertex set
                // rather than the shared pool — `meshopt` packs them that way for compactness.
                let byte = (meshlet.triangle_offset + triangle * 3 + index) as usize;
                let local = mesh.meshlet_triangles[byte] as u32;
                let pooled = mesh.meshlet_vertices[(meshlet.vertex_offset + local) as usize];
                Vec3::from(mesh.vertices[pooled as usize].position)
            };
            let (a, b, c) = (corner(0), corner(1), corner(2));
            let Some(distance) = kooch_core::ray::triangle(origin, direction, a, b, c) else {
                continue;
            };
            if nearest.is_some_and(|hit| hit.distance <= distance) {
                continue;
            }
            nearest = Some(SurfaceHit {
                // 🔴 The face's own normal, not the vertices' shaded ones. A placement wants the
                // plane the triangle actually occupies; an interpolated normal is a lighting
                // convenience and on a low-poly sphere it points somewhere the surface is not.
                normal: (b - a).cross(c - a),
                distance,
            });
        }
    }
    nearest
}

/// How far the surface reaches along `direction`, in the mesh's own space: `max(v · direction)`
/// over every LOD-0 vertex.
///
/// 🔴 A box's reach is not a mesh's reach. Along an axis the two agree, which is exactly why this
/// was invisible until something was set down on a slope: in a diagonal direction a cube's answer
/// is its CORNER, `√3` times a sphere's true radius, and whatever is being placed floats by the
/// difference (#1455).
///
/// `direction` need not be unit length — the result scales with it, which is what lets a caller
/// pass a direction already pushed through a transform.
pub fn support(mesh: &MeshletMesh, direction: Vec3) -> Option<f32> {
    let mut best = f32::NEG_INFINITY;
    for meshlet in mesh.meshlets.iter().filter(|m| m.lod_level == 0) {
        // The meshlet's box reaches at least as far as its vertices do, so a box that cannot beat
        // the running best holds no vertex that can either.
        let min = Vec3::from(meshlet.aabb_min);
        let max = Vec3::from(meshlet.aabb_max);
        let bound = (min + max).dot(direction) * 0.5 + (max - min).abs().dot(direction.abs()) * 0.5;
        if bound <= best {
            continue;
        }
        let first = meshlet.vertex_offset as usize;
        for &pooled in &mesh.meshlet_vertices[first..first + meshlet.vertex_count as usize] {
            let reach = Vec3::from(mesh.vertices[pooled as usize].position).dot(direction);
            best = best.max(reach);
        }
    }
    best.is_finite().then_some(best)
}
