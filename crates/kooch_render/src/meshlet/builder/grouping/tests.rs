use super::*;
use crate::meshlet::asset::{MESHLET_GROUP_NONE, MESHLET_ROOT_PARENT};

/// Builds a single MeshletDescriptor with sensible defaults.
/// Keeps the per-test boilerplate out of the assertions.
fn synthetic_descriptor(vertex_offset: u32, vertex_count: u32) -> MeshletDescriptor {
    MeshletDescriptor {
        vertex_offset,
        triangle_offset: 0,
        vertex_count,
        triangle_count: 0,
        aabb_min: [0.0; 3],
        parent_meshlet_index: MESHLET_ROOT_PARENT,
        aabb_max: [0.0; 3],
        lod_error: 0.0,
        bounds_center: [0.0; 3],
        bounding_radius: 0.0,
        cone_apex: [0.0; 3],
        cone_cutoff: 1.0,
        cone_axis: [0.0, 0.0, 1.0],
        group_index: MESHLET_GROUP_NONE,
        children_group_index: MESHLET_GROUP_NONE,
        lod_level: 0,
        _pad4: 0,
        _pad5: 0,
    }
}

#[test]
fn metis_groups_minimise_cross_partition_edges_on_chain_graph() {
    // Synthetic chain graph: 8 meshlets where adjacent meshlets
    // share exactly one vertex. The optimal 2-partition splits
    // the chain in the middle, cutting exactly one edge.
    //
    // Pool layout: meshlet i covers global verts [i, i+1].
    let mut pool_meshlet_vertices: Vec<u32> = Vec::new();
    let mut prev_meshlets: Vec<MeshletDescriptor> = Vec::new();
    for i in 0..8u32 {
        let off = pool_meshlet_vertices.len() as u32;
        pool_meshlet_vertices.push(i);
        pool_meshlet_vertices.push(i + 1);
        prev_meshlets.push(synthetic_descriptor(off, 2));
    }

    let groups = group_meshlets_metis(&prev_meshlets, &pool_meshlet_vertices, 2);
    assert_eq!(groups.len(), 2, "must produce exactly two non-empty groups");

    // Count cross-partition edges. The chain has 7 edges total;
    // the optimal cut is exactly 1.
    let mut group_of: Vec<usize> = vec![usize::MAX; 8];
    for (g_id, members) in groups.iter().enumerate() {
        for &m in members {
            group_of[m] = g_id;
        }
    }
    let mut cross = 0;
    for i in 0..7 {
        if group_of[i] != group_of[i + 1] {
            cross += 1;
        }
    }
    assert_eq!(
        cross, 1,
        "METIS k-way must find the optimal single-edge cut on a chain graph; \
             got {cross} cross edges with assignment {:?}",
        group_of
    );
}

#[test]
fn metis_groups_isolated_meshlets_round_robin() {
    // No shared vertices anywhere → graph has zero edges.
    // METIS rejects edge-less graphs, so the helper must fall
    // back to round-robin and still produce K non-empty groups.
    let mut pool_meshlet_vertices: Vec<u32> = Vec::new();
    let mut prev_meshlets: Vec<MeshletDescriptor> = Vec::new();
    for i in 0..6u32 {
        let off = pool_meshlet_vertices.len() as u32;
        pool_meshlet_vertices.push(100 + i * 2);
        pool_meshlet_vertices.push(100 + i * 2 + 1);
        prev_meshlets.push(synthetic_descriptor(off, 2));
    }
    let groups = group_meshlets_metis(&prev_meshlets, &pool_meshlet_vertices, 3);
    assert_eq!(
        groups.len(),
        3,
        "round-robin must populate every requested partition"
    );
    let total: usize = groups.iter().map(|g| g.len()).sum();
    assert_eq!(
        total, 6,
        "every meshlet must be assigned to exactly one group"
    );
}

#[test]
fn collect_group_boundary_vertices_flags_only_shared_globals() {
    // Two synthetic meshlets share vertex pool indices 10 and 11.
    // Group A = {meshlet 0}, Group B = {meshlet 1}. Vertices 10
    // and 11 must be flagged as cell-boundary; the rest must not.
    let pool_meshlet_vertices: Vec<u32> = vec![
        // meshlet 0: globals 1, 2, 10, 11
        1, 2, 10, 11, // meshlet 1: globals 5, 6, 10, 11
        5, 6, 10, 11,
    ];
    let prev_meshlets = vec![synthetic_descriptor(0, 4), synthetic_descriptor(4, 4)];
    let groups = vec![vec![0usize], vec![1usize]];
    let boundary = collect_group_boundary_vertices(&groups, &prev_meshlets, &pool_meshlet_vertices);
    assert!(
        boundary.contains(&10),
        "vertex 10 shared between groups must be flagged"
    );
    assert!(
        boundary.contains(&11),
        "vertex 11 shared between groups must be flagged"
    );
    assert!(
        !boundary.contains(&1),
        "vertex 1 (only in group A) must NOT be flagged"
    );
    assert!(
        !boundary.contains(&5),
        "vertex 5 (only in group B) must NOT be flagged"
    );
    assert_eq!(boundary.len(), 2, "exactly 2 shared vertices expected");
}

#[test]
fn collect_group_boundary_vertices_dedups_repeats_inside_a_group() {
    // A vertex appearing N times within the SAME group's meshlets
    // must NOT be counted as cell-boundary unless it ALSO appears
    // in another group.
    let pool_meshlet_vertices: Vec<u32> = vec![
        // meshlet 0: globals 1, 2
        1, 2, // meshlet 1: globals 2, 3 (vertex 2 is intra-group repeat)
        2, 3, // meshlet 2: globals 4, 5 (different group)
        4, 5,
    ];
    let prev_meshlets = vec![
        synthetic_descriptor(0, 2),
        synthetic_descriptor(2, 2),
        synthetic_descriptor(4, 2),
    ];
    let groups = vec![vec![0usize, 1usize], vec![2usize]];
    let boundary = collect_group_boundary_vertices(&groups, &prev_meshlets, &pool_meshlet_vertices);
    assert!(
        boundary.is_empty(),
        "no vertex is actually shared across groups; got {:?}",
        boundary
    );
}
