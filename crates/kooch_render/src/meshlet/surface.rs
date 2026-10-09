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
