//! Multi-LOD chain integration tests. Builds a real chain via `build_meshlets_lod_chain` and
//! asserts the structural invariants the runtime cull/draw path depends on (DAG acyclicity,
//! group-id propagation, offset rebasing, etc.).

use std::collections::HashMap;

use crate::meshlet::asset::{
    DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES, MESHLET_GROUP_NONE, MESHLET_ROOT_PARENT,
};
use crate::meshlet::builder::single_lod::build_default_meshlets;
use crate::meshlet::builder::test_support::make_grid_mesh;

use super::super::lod_config::LodConfig;
use super::build_meshlets_lod_chain;

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
    // Per-group simplify (Nanite-grouped DAG) gives every parent its own lod_error reported by
    // meshopt for that group, so the global error sequence is no longer monotonic across the
    // concatenated chain.
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
fn lod_chain_converges_on_dense_mesh() {
    // a 100×100 grid (~165 LOD-0 meshlets) terminated mid-chain with dozens of roots — the runtime
    // cull never collapses to a single coarse cluster at extreme distance.
    let mesh = make_grid_mesh(100);
    let single = build_default_meshlets(&mesh).expect("single");
    let lod_zero_count = single.meshlets.len();

    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    let roots = chain
        .meshlets
        .iter()
        .filter(|m| m.parent_meshlet_index == MESHLET_ROOT_PARENT)
        .count();
    let max_lod = chain
        .meshlets
        .iter()
        .map(|m| m.lod_level)
        .max()
        .unwrap_or(0);
    assert!(
        roots <= 16,
        "dense planar grid should collapse to ≤ 16 roots; \
         got {roots} roots over {} total meshlets (lod 0 = {lod_zero_count}, max_lod = {max_lod}). \
         Likely the LOD chain terminated early — check whether \
         CELL_BOUNDARY_LOCK_MAX_LEVEL was raised or the builder \
         introduced a new skip path.",
        chain.meshlets.len(),
    );
}

#[test]
fn lod_chain_dag_at_least_one_root_exists() {
    // Per-group DAG: the chain terminates when no group can simplify further.
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
    // Walk from each meshlet up to a root following parent links; assert termination within the
    // chain length (guards against accidental cycles).
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
            chain.meshlets[idx].parent_meshlet_index, MESHLET_ROOT_PARENT,
            "DAG descent from #{i} did not terminate within {max_steps} steps",
        );
    }
}

#[test]
fn lod_chain_dag_group_ids_propagate_to_both_sides() {
    // For every group emitted during chain construction, the group's children share `group_index =
    // id` and the group's parents share `children_group_index = id`.
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
fn lod_chain_dag_parent_lod_error_dominates_children() {
    // `above_too_coarse` and `below_fine` depends on lod_error being monotonically non-decreasing
    // along every parent edge of the DAG.
    let mesh = make_grid_mesh(40);
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("chain");
    for (i, c) in chain.meshlets.iter().enumerate() {
        if c.parent_meshlet_index == MESHLET_ROOT_PARENT {
            continue;
        }
        let p = &chain.meshlets[c.parent_meshlet_index as usize];
        assert!(
            p.lod_error >= c.lod_error,
            "meshlet {i}: parent lod_error {} < child lod_error {} — \
             cut monotonicity violated; the 2-pass selector will let \
             both levels render simultaneously",
            p.lod_error,
            c.lod_error,
        );
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

/// 🔴 A coarse LOD must put the texture where LOD 0 has it: on the shipped capsule, a UV shifted
/// by a fraction of a checker square reads as the pattern cut and slid at the LOD switch (#1149).
#[test]
fn coarse_lods_keep_their_uvs() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/meshes/primitives/capsule.glb"
    ))
    .expect("the capsule ships with the engine");
    let mesh = crate::mesh::parse_mesh_bytes(&bytes).expect("capsule parses");
    let chain = build_meshlets_lod_chain(
        &mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
        LodConfig::default(),
    )
    .expect("lod chain");
    let corner = |vertex: &crate::mesh::MeshVertex| {
        (
            glam::Vec3::from(vertex.position),
            glam::Vec2::from(vertex.uv),
        )
    };
    let fine: Vec<[(glam::Vec3, glam::Vec2); 3]> = mesh
        .indices
        .chunks(3)
        .map(|t| [0, 1, 2].map(|c| corner(&mesh.vertices[t[c] as usize])))
        .collect();
    for meshlet in chain
        .meshlets
        .iter()
        .filter(|m| (1..=3).contains(&m.lod_level))
    {
        for triangle in 0..meshlet.triangle_count as usize {
            let coarse = [0, 1, 2].map(|c| {
                let local =
                    chain.meshlet_triangles[meshlet.triangle_offset as usize + triangle * 3 + c];
                let vertex =
                    chain.meshlet_vertices[meshlet.vertex_offset as usize + local as usize];
                corner(&chain.vertices[vertex as usize])
            });
            let (point, uv) = blend(&coarse, [1.0 / 3.0; 3]);
            let shift = uv_shift(&fine, point, uv);
            // A fifth of a checker square at the capsule material's `uv_scale` (2, 1); 1.0 gave 0.34.
            assert!(
                shift < 0.1,
                "LOD {} shifts the texture by {shift:.3}",
                meshlet.lod_level
            );
        }
    }
}

fn blend(t: &[(glam::Vec3, glam::Vec2); 3], w: [f32; 3]) -> (glam::Vec3, glam::Vec2) {
    (
        t[0].0 * w[0] + t[1].0 * w[1] + t[2].0 * w[2],
        t[0].1 * w[0] + t[1].1 * w[1] + t[2].1 * w[2],
    )
}

/// How far `uv` is from the UV LOD 0 has at the surface point nearest `point`. Among triangles
/// equally near — the two sides of a seam — the closest UV wins.
fn uv_shift(fine: &[[(glam::Vec3, glam::Vec2); 3]], point: glam::Vec3, uv: glam::Vec2) -> f32 {
    let mut best = (f32::MAX, f32::MAX);
    for triangle in fine {
        let weights = nearest_weights(point, triangle.map(|c| c.0));
        let (on, there) = blend(triangle, weights);
        let distance = on.distance(point);
        let shift = ((uv - there) * glam::Vec2::new(2.0, 1.0)).length();
        if distance < best.0 - 1e-4 || (distance < best.0 + 1e-4 && shift < best.1) {
            best = (distance.min(best.0), shift);
        }
    }
    best.1
}

/// Barycentric weights of the point of triangle `t` nearest `p` (Ericson, RTCD 5.1.5).
fn nearest_weights(p: glam::Vec3, [a, b, c]: [glam::Vec3; 3]) -> [f32; 3] {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    [1.0 - v - w, v, w]
}
