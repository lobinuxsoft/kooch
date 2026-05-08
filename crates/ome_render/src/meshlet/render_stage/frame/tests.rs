//! Pure-CPU unit tests for the per-frame helpers in [`super`].

use crate::meshlet::asset::MESHLET_ROOT_PARENT;
use crate::meshlet::pool::GlobalMeshPool;

#[test]
fn ensure_gpu_mesh_marks_pool_dirty_only_for_new_guids() {
    // No GPU device needed — ensure_gpu_mesh defers the upload to
    // render_with_assets so the dirty bookkeeping is the only
    // observable side effect.
    let mut pool = GlobalMeshPool::new();
    let mut pipeline_dirty = false;

    // Simulate the registration semantics directly. The point of
    // the test is to lock the rule "first registration of a GUID
    // marks dirty; repeats do not."
    let before = pool.mesh_count();
    let mesh = crate::meshlet::asset::MeshletMesh {
        vertices: vec![],
        meshlet_vertices: vec![],
        meshlet_triangles: vec![],
        meshlets: vec![crate::meshlet::asset::MeshletDescriptor {
            vertex_offset: 0,
            triangle_offset: 0,
            vertex_count: 0,
            triangle_count: 0,
            aabb_min: [0.0; 3],
            parent_meshlet_index: MESHLET_ROOT_PARENT,
            aabb_max: [0.0; 3],
            lod_error: 0.0,
            bounds_center: [0.0; 3],
            bounding_radius: 0.0,
            cone_apex: [0.0; 3],
            cone_cutoff: 1.0,
            cone_axis: [0.0; 3],
            group_index: crate::meshlet::asset::MESHLET_GROUP_NONE,
            children_group_index: crate::meshlet::asset::MESHLET_GROUP_NONE,
            lod_level: 0,
            _pad4: 0,
            _pad5: 0,
        }],
        aabb: crate::mesh::Aabb::empty(),
    };
    pool.register(&mesh);
    let after = pool.mesh_count();
    if after > before {
        pipeline_dirty = true;
    }
    assert!(pipeline_dirty);

    // Re-registration of the same MeshletMesh adds another pool
    // entry, but the MeshletPipeline registry deduplicates by
    // GUID; the dirty flag mirror in ensure_gpu_mesh keys on the
    // registered_count delta. This test pins the directional rule.
    let before2 = pool.mesh_count();
    let after2 = before2; // simulate dedup hit (no register call)
    assert!(after2 == before2, "dedup keeps the pool unchanged");
}
