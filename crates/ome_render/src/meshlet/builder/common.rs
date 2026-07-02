//! Shared helpers used by both the single-LOD entry point and the
//! multi-LOD chain build: meshopt clusterisation pass + AABB
//! computation. Crate-private — not re-exported from `mod.rs`.

use glam::Vec3;

use crate::mesh::{Aabb, MeshVertex};
use crate::meshlet::asset::{
    MeshletDescriptor, MESHLET_GROUP_NONE, MESHLET_ROOT_PARENT,
};

/// Runs `meshopt::build_meshlets` over `indices` and returns the
/// per-meshlet descriptors plus the per-LOD `meshlet_vertices` and
/// `meshlet_triangles` arrays. `lod_error` tags every descriptor with
/// the simplify error that produced this LOD level (0.0 for LOD 0).
///
/// `parent_meshlet_index`, `group_index`, and `children_group_index`
/// default to their sentinels — the chain build wires the real values
/// when assembling the DAG.
pub(super) fn clusterize_lod(
    indices: &[u32],
    adapter: &meshopt::VertexDataAdapter<'_>,
    vertex_pool: &[MeshVertex],
    max_vertices: usize,
    max_triangles: usize,
    cone_weight: f32,
    lod_error: f32,
) -> (Vec<MeshletDescriptor>, Vec<u32>, Vec<u8>) {
    let raw =
        meshopt::build_meshlets(indices, adapter, max_vertices, max_triangles, cone_weight);

    let mut descriptors = Vec::with_capacity(raw.len());
    for i in 0..raw.len() {
        let m = raw.get(i);
        let bounds = meshopt::compute_meshlet_bounds(m, adapter);
        let ffi = &raw.meshlets[i];
        let (aabb_min, aabb_max) = meshlet_aabb(m, vertex_pool);
        descriptors.push(MeshletDescriptor {
            vertex_offset: ffi.vertex_offset,
            triangle_offset: ffi.triangle_offset,
            vertex_count: ffi.vertex_count,
            triangle_count: ffi.triangle_count,
            aabb_min,
            parent_meshlet_index: MESHLET_ROOT_PARENT,
            aabb_max,
            lod_error,
            bounds_center: bounds.center,
            bounding_radius: bounds.radius,
            cone_apex: bounds.cone_apex,
            cone_cutoff: bounds.cone_cutoff,
            cone_axis: bounds.cone_axis,
            group_index: MESHLET_GROUP_NONE,
            children_group_index: MESHLET_GROUP_NONE,
            lod_level: 0,
            _pad4: 0,
            _pad5: 0,
        });
    }
    (descriptors, raw.vertices, raw.triangles)
}

/// AABB enclosing every vertex in the input. Used to seed the
/// `MeshletMesh.aabb` field when a builder finishes.
pub(super) fn total_aabb(vertices: &[MeshVertex]) -> Aabb {
    let mut aabb = Aabb::empty();
    for v in vertices {
        aabb.expand(Vec3::from_array(v.position));
    }
    aabb
}

/// Per-meshlet AABB, computed from the meshlet's vertex slice. The
/// meshlet's `vertices` slice stores indices INTO the parent mesh's
/// vertex array, so we look up positions there directly.
pub(super) fn meshlet_aabb(
    meshlet: meshopt::Meshlet<'_>,
    vertex_pool: &[MeshVertex],
) -> ([f32; 3], [f32; 3]) {
    let mut mn = [f32::INFINITY; 3];
    let mut mx = [f32::NEG_INFINITY; 3];
    for &pool_idx in meshlet.vertices {
        let p = vertex_pool[pool_idx as usize].position;
        for axis in 0..3 {
            if p[axis] < mn[axis] {
                mn[axis] = p[axis];
            }
            if p[axis] > mx[axis] {
                mx[axis] = p[axis];
            }
        }
    }
    (mn, mx)
}
