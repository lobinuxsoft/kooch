//! Meshlet grouping for the LOD chain build.
//!
//! Two responsibilities:
//! - [`group_meshlets_metis`] partitions the previous LOD's meshlets
//!   into groups by minimising the shared-vertex edge cut on the
//!   connectivity graph (Karis SIGGRAPH 2021).
//! - [`collect_group_boundary_vertices`] returns the set of mesh-pool
//!   vertex indices that end up on a cell boundary, so the per-group
//!   simplify can lock them and adjacent cells stitch identically
//!   along the shared border (Ponchio §3.4.3).

use std::collections::{HashMap, HashSet};

use crate::meshlet::asset::MeshletDescriptor;

/// Walks every group and returns the set of mesh-pool vertex indices
/// that appear in ≥ 2 groups. Those vertices live on a cell boundary
/// and must be locked during the per-group simplify so adjacent cells
/// collapse identically along the shared border.
///
/// Without this lock, two neighbouring groups can collapse the same
/// boundary edge to different parent vertices — Ponchio §3.4.3
/// (boundary management problem). The visible symptom is holes /
/// Z-fighting strips along cell seams in coarse LODs.
pub(super) fn collect_group_boundary_vertices(
    groups: &[Vec<usize>],
    prev_lod_meshlets: &[MeshletDescriptor],
    pool_meshlet_vertices: &[u32],
) -> HashSet<u32> {
    // For each pool vertex, the count of distinct groups that touch
    // it. We only need "touched by ≥ 2 distinct groups", so a single
    // u8 counter (saturating at 2) is enough.
    let mut touched_by_n_groups: HashMap<u32, u8> = HashMap::new();
    for group in groups {
        // Dedupe inside the group first so a vertex shared by N
        // sibling meshlets in the SAME group still counts as +1.
        let mut group_vertices: HashSet<u32> = HashSet::new();
        for &meshlet_idx in group {
            let m = &prev_lod_meshlets[meshlet_idx];
            let base = m.vertex_offset as usize;
            for i in 0..m.vertex_count as usize {
                group_vertices.insert(pool_meshlet_vertices[base + i]);
            }
        }
        for v in group_vertices {
            let counter = touched_by_n_groups.entry(v).or_insert(0);
            *counter = counter.saturating_add(1);
        }
    }
    touched_by_n_groups
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(v, _)| v)
        .collect()
}

/// METIS graph-partition grouping (Karis SIGGRAPH 2021 / Ponchio §3.5,
/// the actually-correct path). Builds a graph whose:
/// - **nodes** are meshlets at the previous LOD,
/// - **edges** join meshlets that share ≥ 1 mesh-pool vertex,
/// - **edge weight** = number of vertices the two meshlets share.
///
/// Then calls METIS k-way multilevel partitioning to split the graph
/// into `target_groups` parts while **minimising the edge cut**
/// (sum of weights of cross-partition edges). Result: groups whose
/// internal connectivity is dense and whose shared-vertex border with
/// neighbours is as small as possible — leaves the per-group simplify
/// the maximum interior surface to reduce while keeping the locked
/// boundary minimal.
///
/// History: replaces the spatial Voronoi grouping (#469 V1) and the
/// Morton chunker before it (#465 V1). Spatial schemes ignore
/// topology — two meshlets can sit close in space without being
/// connected (opposite sides of thin geometry, disjoint shells, etc.)
/// and end up sharing many "accidental" boundary edges; coarse LODs
/// then go full of holes. Confirmed by inspecting the two open-source
/// state-of-art implementations: `Scthe/nanite-webgpu` and
/// `pettett/multires` — both METIS.
///
/// Edge cases:
/// - `target_groups < 2` → single group (METIS rejects k=1).
/// - Fewer meshlets than `target_groups` → cap k at meshlet count.
/// - Zero edges (every meshlet isolated) → round-robin fallback,
///   METIS errors out on edge-less input.
pub(super) fn group_meshlets_metis(
    meshlets: &[MeshletDescriptor],
    pool_meshlet_vertices: &[u32],
    target_groups: usize,
) -> Vec<Vec<usize>> {
    let n = meshlets.len();
    if n == 0 {
        return Vec::new();
    }
    let k = target_groups.min(n).max(1);
    if k == 1 {
        return vec![(0..n).collect()];
    }

    // Build vertex → meshlets index. A vertex shared by M meshlets
    // generates M·(M-1)/2 graph edges (one per pair).
    let mut vertex_to_meshlets: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, m) in meshlets.iter().enumerate() {
        let base = m.vertex_offset as usize;
        for v_off in 0..m.vertex_count as usize {
            let global_v = pool_meshlet_vertices[base + v_off];
            vertex_to_meshlets.entry(global_v).or_default().push(i);
        }
    }

    // Accumulate pair → shared-vertex count.
    let mut pair_weight: HashMap<(usize, usize), i32> = HashMap::new();
    for owners in vertex_to_meshlets.values() {
        // Dedup: a single meshlet could legitimately list a vertex
        // twice if its meshlet_vertices entry was duplicated (rare
        // but defensive).
        let mut owners = owners.clone();
        owners.sort_unstable();
        owners.dedup();
        for a in 0..owners.len() {
            for b in (a + 1)..owners.len() {
                let key = (owners[a], owners[b]);
                *pair_weight.entry(key).or_insert(0) += 1;
            }
        }
    }

    // Convert to CSR for METIS. Each undirected edge contributes
    // entries on BOTH endpoints' adjacency lists.
    let mut adj: Vec<Vec<(i32, i32)>> = vec![Vec::new(); n];
    for ((i, j), w) in pair_weight {
        adj[i].push((j as i32, w));
        adj[j].push((i as i32, w));
    }

    let mut xadj: Vec<i32> = Vec::with_capacity(n + 1);
    let mut adjncy: Vec<i32> = Vec::new();
    let mut adjwgt: Vec<i32> = Vec::new();
    xadj.push(0);
    for neighbours in &adj {
        for &(j, w) in neighbours {
            adjncy.push(j);
            adjwgt.push(w);
        }
        xadj.push(adjncy.len() as i32);
    }

    if adjncy.is_empty() {
        return round_robin(n, k);
    }

    let mut part = vec![0i32; n];
    let metis_result = metis::Graph::new(1, k as i32, &xadj, &adjncy)
        .ok()
        .and_then(|g| g.set_adjwgt(&adjwgt).part_kway(&mut part).ok());
    if metis_result.is_none() {
        // METIS hit some internal limit — degrade gracefully so the
        // build still produces SOMETHING rather than panicking.
        // Should be unreachable on well-formed input.
        return round_robin(n, k);
    }

    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); k];
    for (i, p) in part.iter().enumerate() {
        groups[*p as usize].push(i);
    }
    groups.retain(|g| !g.is_empty());
    groups
}

fn round_robin(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); k];
    for i in 0..n {
        groups[i % k].push(i);
    }
    groups.retain(|g| !g.is_empty());
    groups
}

#[cfg(test)]
mod tests {
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
        assert_eq!(groups.len(), 3, "round-robin must populate every requested partition");
        let total: usize = groups.iter().map(|g| g.len()).sum();
        assert_eq!(total, 6, "every meshlet must be assigned to exactly one group");
    }

    #[test]
    fn collect_group_boundary_vertices_flags_only_shared_globals() {
        // Two synthetic meshlets share vertex pool indices 10 and 11.
        // Group A = {meshlet 0}, Group B = {meshlet 1}. Vertices 10
        // and 11 must be flagged as cell-boundary; the rest must not.
        let pool_meshlet_vertices: Vec<u32> = vec![
            // meshlet 0: globals 1, 2, 10, 11
            1, 2, 10, 11,
            // meshlet 1: globals 5, 6, 10, 11
            5, 6, 10, 11,
        ];
        let prev_meshlets = vec![synthetic_descriptor(0, 4), synthetic_descriptor(4, 4)];
        let groups = vec![vec![0usize], vec![1usize]];
        let boundary = collect_group_boundary_vertices(
            &groups,
            &prev_meshlets,
            &pool_meshlet_vertices,
        );
        assert!(boundary.contains(&10), "vertex 10 shared between groups must be flagged");
        assert!(boundary.contains(&11), "vertex 11 shared between groups must be flagged");
        assert!(!boundary.contains(&1), "vertex 1 (only in group A) must NOT be flagged");
        assert!(!boundary.contains(&5), "vertex 5 (only in group B) must NOT be flagged");
        assert_eq!(boundary.len(), 2, "exactly 2 shared vertices expected");
    }

    #[test]
    fn collect_group_boundary_vertices_dedups_repeats_inside_a_group() {
        // A vertex appearing N times within the SAME group's meshlets
        // must NOT be counted as cell-boundary unless it ALSO appears
        // in another group.
        let pool_meshlet_vertices: Vec<u32> = vec![
            // meshlet 0: globals 1, 2
            1, 2,
            // meshlet 1: globals 2, 3 (vertex 2 is intra-group repeat)
            2, 3,
            // meshlet 2: globals 4, 5 (different group)
            4, 5,
        ];
        let prev_meshlets = vec![
            synthetic_descriptor(0, 2),
            synthetic_descriptor(2, 2),
            synthetic_descriptor(4, 2),
        ];
        let groups = vec![vec![0usize, 1usize], vec![2usize]];
        let boundary = collect_group_boundary_vertices(
            &groups,
            &prev_meshlets,
            &pool_meshlet_vertices,
        );
        assert!(
            boundary.is_empty(),
            "no vertex is actually shared across groups; got {:?}",
            boundary
        );
    }
}
