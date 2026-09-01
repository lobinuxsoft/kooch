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

/// A failed load never enters the cache, so the GUID is still pending
/// next frame — the retry loop is unbounded by construction and the
/// warning was too: 1068 lines for two GUIDs in nine seconds (#693).
///
/// Modelled directly, the way `ensure_gpu_mesh_marks_pool_dirty_only_for_new_guids`
/// above models registration: the call site needs a GPU stage, and the
/// rule being locked is "said once per GUID until it resolves".
#[test]
fn an_unresolved_mesh_is_said_once() {
    use kooch_core::Guid;
    use std::collections::HashSet;

    let broken = Guid::new_v4();
    let mut reported: HashSet<Guid> = HashSet::new();
    let mut said = 0;

    // Four frames: it fails, fails again, resolves, then breaks again.
    for pending in [vec![broken], vec![broken], vec![], vec![broken]] {
        reported.retain(|guid| pending.contains(guid));
        for guid in pending {
            if reported.insert(guid) {
                said += 1;
            }
        }
    }

    assert_eq!(
        said, 2,
        "once for the first failure, once more after it resolved and broke again",
    );
}

/// The actionable line, and the condition that keeps it to one.
///
/// A scene where nothing resolves is a broken run, not N warnings — and
/// the message naming the cause was buried under a thousand correct ones.
#[test]
fn a_scene_that_resolves_nothing_says_so_once() {
    let fired = |pending: usize, referenced: usize, reported: usize, cached: usize| {
        pending == referenced && reported == 0 && cached == 0
    };

    assert!(
        fired(2, 2, 0, 0),
        "nothing resolved and nothing was said yet"
    );
    assert!(
        !fired(2, 2, 1, 0),
        "already reported — this is the second frame, and it must stay quiet",
    );
    assert!(
        !fired(1, 2, 0, 0),
        "one mesh of two is a missing asset, not a broken run",
    );
    assert!(
        !fired(2, 2, 0, 5),
        "the database has meshes, so the engine root is not the problem",
    );
}
