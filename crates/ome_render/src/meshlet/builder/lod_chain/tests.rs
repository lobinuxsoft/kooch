//! Multi-LOD chain integration tests. Builds a real chain via
//! `build_meshlets_lod_chain` and asserts the structural invariants
//! the runtime cull/draw path depends on (DAG acyclicity, group-id
//! propagation, offset rebasing, etc.).

use std::collections::HashMap;

use crate::meshlet::asset::{
    MESHLET_GROUP_NONE, MESHLET_ROOT_PARENT, DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES,
};
use crate::meshlet::builder::single_lod::build_default_meshlets;
use crate::meshlet::builder::test_support::make_grid_mesh;

use super::build_meshlets_lod_chain;
use super::super::lod_config::LodConfig;

#[test]
fn lod_chain_lod_zero_has_zero_error() {
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("lod chain");
    // The first meshlet must come from LOD 0 (error 0.0). Chain
    // never reorders; LOD 0 always lands first.
    assert_eq!(chain.meshlets[0].lod_error, 0.0);
}

#[test]
fn lod_chain_lod_zero_meshlets_appear_first() {
    // Per-group simplify (Nanite-grouped DAG) gives every parent its
    // own lod_error reported by meshopt for that group, so the global
    // error sequence is no longer monotonic across the concatenated
    // chain. The structural invariant that survives: every LOD 0
    // meshlet (error == 0.0) lands before any LOD ≥ 1 meshlet
    // (error > 0.0) because LOD 0 is appended in one global pass
    // before the per-group loop runs.
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("lod chain");
    let lod_zero_count = chain
        .meshlets
        .iter()
        .take_while(|m| m.lod_error == 0.0)
        .count();
    assert!(lod_zero_count > 0, "must have at least one LOD 0 meshlet");
    for m in chain.meshlets.iter().skip(lod_zero_count) {
        assert!(
            m.lod_error > 0.0,
            "all meshlets after the LOD 0 prefix must carry simplify error > 0",
        );
    }
}

#[test]
fn lod_chain_produces_more_meshlets_than_single_lod() {
    let mesh = make_grid_mesh(20);
    let single = build_default_meshlets(&mesh).expect("single");
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    assert!(
        chain.meshlets.len() > single.meshlets.len(),
        "lod chain ({}) should add meshlets beyond LOD 0 ({})",
        chain.meshlets.len(),
        single.meshlets.len(),
    );
}

#[test]
fn lod_chain_offsets_stay_within_pool_bounds() {
    // Concatenation must rebase per-LOD offsets correctly so the
    // GPU can index a single flat pool.
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    for (i, m) in chain.meshlets.iter().enumerate() {
        let v_end = m.vertex_offset + m.vertex_count;
        assert!(
            v_end as usize <= chain.meshlet_vertices.len(),
            "meshlet {i} vertex range exceeds pool: {v_end} > {}",
            chain.meshlet_vertices.len()
        );
        // triangle_offset is in bytes; each triangle is 3 bytes.
        let t_end_bytes = m.triangle_offset + m.triangle_count * 3;
        assert!(
            t_end_bytes as usize <= chain.meshlet_triangles.len(),
            "meshlet {i} triangle range exceeds pool: {t_end_bytes} > {}",
            chain.meshlet_triangles.len()
        );
    }
}

#[test]
fn lod_chain_dag_at_least_one_root_exists() {
    // Per-group DAG: the chain terminates when no group can simplify
    // further. Every meshlet that did not get a parent assigned during
    // the loop is left at MESHLET_ROOT_PARENT; the chain must end
    // with at least one such terminal node so the runtime selector
    // has somewhere to stop descending.
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    let root_count = chain
        .meshlets
        .iter()
        .filter(|m| m.parent_meshlet_index == MESHLET_ROOT_PARENT)
        .count();
    assert!(
        root_count > 0,
        "chain must contain at least one root meshlet (parent sentinel)",
    );
}

#[test]
fn lod_chain_dag_parents_point_into_chain_bounds() {
    // Every non-root parent_meshlet_index references a real meshlet
    // that lives later in the chain (parents are appended after
    // children).
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    for (i, m) in chain.meshlets.iter().enumerate() {
        if m.parent_meshlet_index == MESHLET_ROOT_PARENT {
            continue;
        }
        let p = m.parent_meshlet_index as usize;
        assert!(
            p < chain.meshlets.len(),
            "child #{i} parent index {p} out of bounds (chain has {})",
            chain.meshlets.len(),
        );
        assert!(
            p > i,
            "parent #{p} must appear after child #{i} in the chain",
        );
    }
}

#[test]
fn lod_chain_dag_is_acyclic_via_descent_terminates() {
    // Walk from each meshlet up to a root following parent links;
    // assert termination within the chain length (guards against
    // accidental cycles). The grouped DAG always appends parents
    // strictly after their children in the chain, so length is a safe
    // upper bound on the descent depth.
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    let max_steps = chain.meshlets.len() + 1;
    for (i, _) in chain.meshlets.iter().enumerate() {
        let mut idx = i;
        for _ in 0..max_steps {
            let parent = chain.meshlets[idx].parent_meshlet_index;
            if parent == MESHLET_ROOT_PARENT {
                break;
            }
            idx = parent as usize;
        }
        assert_eq!(
            chain.meshlets[idx].parent_meshlet_index,
            MESHLET_ROOT_PARENT,
            "DAG descent from #{i} did not terminate within {max_steps} steps",
        );
    }
}

#[test]
fn lod_chain_dag_group_ids_propagate_to_both_sides() {
    // For every group emitted during chain construction, the group's
    // children share `group_index = id` and the group's parents share
    // `children_group_index = id`. Validates the 2-pass cull contract:
    // every meshlet that points at a real parent_meshlet_index also
    // has a real group_index, and vice versa for parent meshlets that
    // own children below.
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    for m in &chain.meshlets {
        // A meshlet that has a parent must also have a group.
        if m.parent_meshlet_index != MESHLET_ROOT_PARENT {
            assert_ne!(
                m.group_index, MESHLET_GROUP_NONE,
                "meshlet with parent must have a non-NONE group_index",
            );
        }
    }
    // Every meshlet referenced as a parent_meshlet_index by some child
    // must carry a non-NONE children_group_index.
    for m in &chain.meshlets {
        if m.parent_meshlet_index == MESHLET_ROOT_PARENT {
            continue;
        }
        let parent = &chain.meshlets[m.parent_meshlet_index as usize];
        assert_ne!(
            parent.children_group_index, MESHLET_GROUP_NONE,
            "any meshlet referenced as a parent must have a non-NONE children_group_index",
        );
    }
}

#[test]
fn lod_chain_dag_group_siblings_share_group_id() {
    // All children pointing at parents with the same
    // children_group_index must themselves share the same group_index.
    // The 2-pass cull's atomicMax convergence depends on this.
    let mesh = make_grid_mesh(20);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    let mut group_of_children: HashMap<u32, u32> = HashMap::new();
    for m in &chain.meshlets {
        if m.parent_meshlet_index == MESHLET_ROOT_PARENT {
            continue;
        }
        let parent = &chain.meshlets[m.parent_meshlet_index as usize];
        let parents_group = parent.children_group_index;
        if let Some(&prev_group) = group_of_children.get(&parents_group) {
            assert_eq!(
                prev_group, m.group_index,
                "children sharing a parents-group must share group_index",
            );
        } else {
            group_of_children.insert(parents_group, m.group_index);
        }
    }
}

#[test]
fn lod_chain_caps_at_max_levels() {
    // max_levels controls how many descent passes the per-group loop
    // runs. Compare two chains: a tighter cap must produce ≤ the
    // meshlet count of a looser cap.
    let mesh = make_grid_mesh(40);
    let chain_low = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig {
            max_levels: 2,
            ..Default::default()
        },
    )
    .expect("low");
    let chain_high = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig {
            max_levels: 6,
            ..Default::default()
        },
    )
    .expect("high");
    assert!(
        chain_low.meshlets.len() <= chain_high.meshlets.len(),
        "tighter max_levels ({}) must produce ≤ meshlets than the looser one ({})",
        chain_low.meshlets.len(),
        chain_high.meshlets.len(),
    );
}
