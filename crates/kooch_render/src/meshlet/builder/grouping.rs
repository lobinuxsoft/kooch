//! Meshlet grouping for the LOD chain build.

use std::collections::{HashMap, HashSet};

use crate::meshlet::asset::MeshletDescriptor;

/// Walks every group and returns the set of mesh-pool vertex indices that appear in ≥ 2 groups.
/// Those vertices live on a cell boundary and must be locked during the per-group simplify so
/// adjacent cells collapse identically along the shared border.
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

/// METIS graph-partition grouping (Karis SIGGRAPH 2021 / Ponchio §3.5, the actually-correct path).
/// Builds a graph whose: - **nodes** are meshlets at the previous LOD, - **edges** join meshlets
/// that share ≥ 1 mesh-pool vertex, - **edge weight** = number of vertices the two meshlets share.
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
mod tests;
