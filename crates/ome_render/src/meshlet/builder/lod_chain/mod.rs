//! Multi-LOD Nanite-grouped DAG chain build. Concatenates every LOD's
//! meshlets into one descriptor array, rebasing per-LOD
//! `vertex_offset` / `triangle_offset` so the GPU can index a single
//! flat buffer.
//!
//! See [`build_meshlets_lod_chain`] for the per-level algorithm.

use std::collections::HashMap;

use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::asset::MeshletDescriptor;
use crate::meshlet::asset::MeshletMesh;

use super::common::{clusterize_lod, total_aabb};
use super::error::MeshletBuildError;
use super::grouping::{collect_group_boundary_vertices, group_meshlets_metis};
use super::lod_config::LodConfig;

/// Number of children grouped together when building each LOD level
/// of the Nanite-style DAG. Karis SIGGRAPH 2021 used 4 — small enough
/// that simplify reliably collapses the group to a parent meshlet
/// (≤ MAX_TRIANGLES), large enough to amortise the per-group
/// overhead.
pub(super) const NANITE_GROUP_SIZE: usize = 4;

/// Maximum parent meshlets a single group may emit (#535). The
/// strict `≤ NANITE_GROUP_SIZE` bound rejected groups whose
/// simplification reduced triangle count but whose re-clusterisation
/// fragmented into 5-8 small clusters (commonly when the dropped
/// cell-boundary lock past level 3 lets meshopt simplify aggressively
/// → sparser geometry → meshopt::build_meshlets emits more, smaller
/// clusters). Those groups had a triangle-space reduction but no
/// cluster-space reduction, so the strict bound counted them as
/// "expansion" and dropped the children to root status.
///
/// Doubling the bound to 2× NANITE_GROUP_SIZE accepts the temporary
/// cluster-count growth in exchange for the triangle-space progress.
/// The chain converges in subsequent levels because the extra
/// parents get re-grouped and re-simplified next iteration. Bevy's
/// meshlet builder uses a similar relaxed bound.
const PARENTS_PER_GROUP_CAP: usize = NANITE_GROUP_SIZE * 2;

/// LOD level past which per-vertex cell-boundary locks are dropped
/// (#535). Beyond this depth the chain's groups are spatially small,
/// the boundary-to-interior vertex ratio approaches 1, and meshopt's
/// `simplify_with_locks` runs out of legal collapses — every group
/// returns its input unchanged and stays rooted at the previous
/// level. The user's diagnostic run on TEST3 showed 80 % of groups
/// failing simplification at level 5 with locks active.
///
/// Karis's solution (Nanite SIGGRAPH 2021 §5.2): keep the locks on
/// the first few levels where seam tearing would be visible, drop
/// them once the cluster footprint goes sub-pixel. We pick 3 because
/// at level 4+ a cluster's projected footprint at the camera distance
/// where it gets selected is smaller than a single pixel — a torn
/// seam there is invisible. Mesh-edge topology is still protected by
/// `meshopt::SimplifyOptions::LockBorder`.
const CELL_BOUNDARY_LOCK_MAX_LEVEL: u32 = 3;

/// Builds a multi-LOD [`MeshletMesh`] using the Nanite-grouped DAG
/// algorithm. The resulting `MeshletMesh` concatenates every LOD's
/// meshlets into one descriptor array, rebasing per-LOD
/// `vertex_offset` / `triangle_offset` so the GPU can index a single
/// flat buffer.
///
/// # Algorithm (per LOD level)
///
/// 1. Take the previous LOD's meshlets.
/// 2. Build the meshlet connectivity graph (nodes = meshlets, edges
///    weighted by shared-vertex count) and partition it with METIS
///    k-way multilevel partitioning into ~prev_count / GROUP_SIZE
///    groups. Minimising edge-cut directly minimises the shared
///    border each group will have to lock during simplify.
/// 3. Identify the cell-boundary vertex set (vertices touched by
///    ≥ 2 groups) so the per-group simplify call below can lock
///    them — adjacent groups must collapse the shared border
///    identically (Ponchio §3.4.3 boundary management).
/// 4. For each group:
///    - Resolve the group's underlying triangles into a local
///      vertex-deduped sub-mesh + per-vertex lock mask.
///    - Run `meshopt::simplify_with_locks` with `LockBorder` so the
///      mesh's topological edges AND the cell border both survive
///      the collapse — adjacent groups stitch seamlessly.
///    - Re-cluster the simplified triangles into ≤ NANITE_GROUP_SIZE
///      parent meshlets covering the children's region.
///    - Wire each child's `parent_meshlet_index` to the new parent.
/// 5. Stop when no group could simplify further (topology lock).
///
/// LOD 0 is always present (clusterised once, no simplification).
/// `lod_error` on each meshlet carries the simplify error reported by
/// `meshopt`; LOD 0 stays at 0.0.
pub fn build_meshlets_lod_chain(
    mesh: &Mesh,
    max_vertices: usize,
    max_triangles: usize,
    cone_weight: f32,
    lod_config: LodConfig,
) -> Result<MeshletMesh, MeshletBuildError> {
    if mesh.indices.is_empty() {
        return Err(MeshletBuildError::EmptyMesh);
    }

    let vertex_stride = std::mem::size_of::<MeshVertex>();
    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let adapter = meshopt::VertexDataAdapter::new(vertex_bytes, vertex_stride, 0)?;
    let mesh_aabb = total_aabb(&mesh.vertices);

    // LOD 0 — full detail, single global cluster pass.
    let (mut all_descriptors, mut all_meshlet_vertices, mut all_meshlet_triangles) =
        clusterize_lod(
            &mesh.indices,
            &adapter,
            &mesh.vertices,
            max_vertices,
            max_triangles,
            cone_weight,
            0.0,
        );

    let lod_zero_count = all_descriptors.len();
    tracing::info!(
        target: "ome_render::meshlet::builder",
        lod_zero_count,
        "LOD chain build: starting"
    );
    // Per-level input is a list of pool indices, not a contiguous
    // range. The contiguous form (prev_start..prev_end) assumed every
    // member of the previous level got either a parent or stayed
    // rooted forever. Plan C (#535) lets children whose group failed
    // to simplify *retry* in subsequent levels by being mixed back
    // into the next METIS partitioning alongside freshly emitted
    // parents. This breaks the `lod_level == chain depth` invariant
    // (a retried child still carries its original lod_level), but the
    // runtime selector keys on group_max_err, not lod_level, so DAG
    // descent stays correct. The cost is that LOD-level histograms
    // and `MeshInstance.lod_force_level` filters report the pre-
    // simplification level of orphans, not the level at which they
    // eventually paired with a parent.
    let mut prev_lod_indices: Vec<u32> = (0..lod_zero_count as u32).collect();
    let mut current_error = lod_config.initial_error;
    // Sequential id assigned per group as the chain is built. Every
    // (children, parents) pair from the same simplification step
    // shares the same group_id — children store it as `group_index`
    // ("the group I'm a child of"), parents store it as
    // `children_group_index` ("the group I'm a parent of"). The
    // 2-pass cull (#465) keys group_max_err by this id.
    let mut next_group_id: u32 = 0;

    for level in 1..lod_config.max_levels {
        // The chain depth for parents emitted in this iteration.
        // LOD 0 sits at level 0; parents from the first simplify are
        // level 1; etc. The LOD-stack inspector (#467) and
        // MeshInstance.lod_force_level filter by this value.
        let parent_lod_level = level as u32;
        let prev_count = prev_lod_indices.len();
        if prev_count <= 1 {
            break; // Single meshlet at the previous level → cannot group.
        }

        // Snapshot the previous level's meshlets so the group loop
        // can read them by local index while parents get appended to
        // the same all_descriptors vector. `prev_lod_indices[i]` maps
        // a group-local meshlet position back to its pool index.
        let prev_meshlets: Vec<MeshletDescriptor> = prev_lod_indices
            .iter()
            .map(|&pool_idx| all_descriptors[pool_idx as usize])
            .collect();

        let target_groups = (prev_count / NANITE_GROUP_SIZE).max(1);
        let groups =
            group_meshlets_metis(&prev_meshlets, &all_meshlet_vertices, target_groups);
        let group_boundary_globals =
            collect_group_boundary_vertices(&groups, &prev_meshlets, &all_meshlet_vertices);

        // Accumulator for the *next* level's input. Receives:
        //   - new parents emitted in this level (the happy path)
        //   - orphaned children whose group failed to simplify
        //     (Plan C re-injection — they get another shot when METIS
        //     groups them with neighbours next level)
        let mut next_prev_lod_indices: Vec<u32> = Vec::with_capacity(prev_count);
        let mut any_group_emitted_parent = false;
        // Per-level instrumentation (#535). Counts the funnel from
        // METIS partitioning down to actually-emitted parents so the
        // root explosion can be diagnosed without re-running with a
        // CPU profiler.
        let mut groups_skipped_too_few_tris = 0u32;
        let mut groups_skipped_target_too_small = 0u32;
        let mut groups_skipped_no_simplification = 0u32;
        let mut groups_skipped_clusterize_overflow = 0u32;
        let mut groups_emitted = 0u32;
        let mut parents_emitted = 0u32;
        let mut children_orphaned = 0u32;

        for group in &groups {
            let geo = extract_group_geometry(
                group,
                &prev_meshlets,
                &all_meshlet_vertices,
                &all_meshlet_triangles,
                &mesh.vertices,
            );
            // Build the lock mask paralleling `geo.vertices`. At
            // shallow LOD levels (1..=CELL_BOUNDARY_LOCK_MAX_LEVEL)
            // any group-local vertex whose original mesh-pool index
            // is in the cell-boundary set must survive simplification
            // so adjacent groups remain stitched. At deeper levels
            // the lock is dropped — see the constant's docs for the
            // Karis trade-off rationale.
            let vertex_lock: Vec<bool> = if parent_lod_level <= CELL_BOUNDARY_LOCK_MAX_LEVEL {
                geo.global_indices
                    .iter()
                    .map(|gi| group_boundary_globals.contains(gi))
                    .collect()
            } else {
                vec![false; geo.global_indices.len()]
            };
            // Need at least two triangles to produce a meaningful
            // simplification budget; orphan back into next level so
            // METIS can re-pair them with bulkier neighbours.
            if geo.indices.len() < 6 {
                groups_skipped_too_few_tris += 1;
                children_orphaned += group.len() as u32;
                for &child_local in group {
                    next_prev_lod_indices.push(prev_lod_indices[child_local]);
                }
                continue;
            }

            let group_vertex_bytes: &[u8] = bytemuck::cast_slice(&geo.vertices);
            let group_adapter =
                meshopt::VertexDataAdapter::new(group_vertex_bytes, vertex_stride, 0)?;

            // Single-pass simplify: aim at half the indices, accept
            // whatever clusterize emits up to NANITE_GROUP_SIZE
            // parents.
            let target_count = ((max_triangles * 3) as usize)
                .min(((geo.indices.len() as f32) * lod_config.target_ratio) as usize);
            let target_count = (target_count / 3) * 3;
            if target_count < 3 {
                groups_skipped_target_too_small += 1;
                children_orphaned += group.len() as u32;
                for &child_local in group {
                    next_prev_lod_indices.push(prev_lod_indices[child_local]);
                }
                continue;
            }

            let mut actual_error = 0.0f32;
            // simplify_with_locks: explicit per-vertex locks force
            // cell-boundary vertices to survive collapse, complementing
            // LockBorder which only handles topological mesh borders.
            let simplified = meshopt::simplify_with_locks(
                &geo.indices,
                &group_adapter,
                &vertex_lock,
                target_count,
                current_error,
                meshopt::SimplifyOptions::LockBorder,
                Some(&mut actual_error),
            );
            if simplified.is_empty() || simplified.len() >= geo.indices.len() {
                // No reduction possible for this group. Plan C: re-
                // inject the children into the next level's input so
                // METIS re-partitions them with neighbours that *did*
                // simplify. Without re-injection the children would
                // stay rooted forever, producing the three-digit root
                // explosion seen on TEST3 mesh 1 and 5 (#535).
                groups_skipped_no_simplification += 1;
                children_orphaned += group.len() as u32;
                for &child_local in group {
                    next_prev_lod_indices.push(prev_lod_indices[child_local]);
                }
                continue;
            }

            let (parent_descs, parent_mlv_local, parent_mlt) = clusterize_lod(
                &simplified,
                &group_adapter,
                &geo.vertices,
                max_vertices,
                max_triangles,
                cone_weight,
                actual_error,
            );
            if parent_descs.is_empty() || parent_descs.len() > PARENTS_PER_GROUP_CAP {
                // Empty: simplify produced nothing usable. Cap
                // exceeded: even the relaxed bound of 2×
                // NANITE_GROUP_SIZE rejected — the geometry
                // genuinely exploded into too many small clusters
                // for this group to count as progress. Re-inject
                // children so a re-partitioning may yield a healthier
                // group next level.
                groups_skipped_clusterize_overflow += 1;
                children_orphaned += group.len() as u32;
                for &child_local in group {
                    next_prev_lod_indices.push(prev_lod_indices[child_local]);
                }
                continue;
            }

            // Pad triangles to a u32 boundary before appending —
            // the cull shader reads them as array<u32>.
            while all_meshlet_triangles.len() % 4 != 0 {
                all_meshlet_triangles.push(0);
            }
            let vertex_offset_base = all_meshlet_vertices.len() as u32;
            let triangle_offset_base = all_meshlet_triangles.len() as u32;

            // Remap meshlet_vertices values from group-local space
            // back to the mesh-global vertex pool.
            let remapped_mlv: Vec<u32> = parent_mlv_local
                .iter()
                .map(|&local_idx| geo.global_indices[local_idx as usize])
                .collect();

            // Allocate the group id BEFORE pushing parents so we can
            // stamp it on both sides of the relationship below.
            let group_id = next_group_id;
            next_group_id += 1;

            let first_parent_pool_idx = all_descriptors.len() as u32;
            for desc in &parent_descs {
                all_descriptors.push(MeshletDescriptor {
                    vertex_offset: desc.vertex_offset + vertex_offset_base,
                    triangle_offset: desc.triangle_offset + triangle_offset_base,
                    children_group_index: group_id,
                    lod_level: parent_lod_level,
                    ..*desc
                });
            }
            all_meshlet_vertices.extend(remapped_mlv);
            all_meshlet_triangles.extend(parent_mlt);

            // Wire each child to the parent meshlet (within this
            // group's emitted set) whose `bounds_center` is closest
            // to the child's. Confines parent-child coherence to the
            // group, so the proximity-DAG tearing case (children of
            // group X pointing at parents from group Y) is
            // structurally impossible — and every emitted parent is
            // referenced by at least its closest child, so the
            // selector cannot leave a parent rendering as a stray
            // root on top of its descended siblings.
            //
            // Children also receive `group_index = group_id`. All
            // siblings in the group share it, so pass 1's atomicMax
            // converges to the true per-group parent_err max
            // regardless of which closest-parent each child picked.
            for &child_local in group {
                let cc = prev_meshlets[child_local].bounds_center;
                let mut best: (f32, u32) = (f32::INFINITY, first_parent_pool_idx);
                for (i, p) in parent_descs.iter().enumerate() {
                    let pc = p.bounds_center;
                    let dx = pc[0] - cc[0];
                    let dy = pc[1] - cc[1];
                    let dz = pc[2] - cc[2];
                    let d2 = dx * dx + dy * dy + dz * dz;
                    if d2 < best.0 {
                        best = (d2, first_parent_pool_idx + i as u32);
                    }
                }
                let child_pool_idx = prev_lod_indices[child_local] as usize;
                let child = &mut all_descriptors[child_pool_idx];
                child.parent_meshlet_index = best.1;
                child.group_index = group_id;
            }

            // Push the freshly emitted parents into the next level's
            // input so they participate in METIS partitioning alongside
            // any orphans re-injected from skipped groups above.
            for i in 0..parent_descs.len() {
                next_prev_lod_indices.push(first_parent_pool_idx + i as u32);
            }

            any_group_emitted_parent = true;
            groups_emitted += 1;
            parents_emitted += parent_descs.len() as u32;
        }

        tracing::info!(
            target: "ome_render::meshlet::builder",
            level = parent_lod_level,
            prev_count,
            target_groups,
            actual_groups = groups.len(),
            groups_emitted,
            parents_emitted,
            children_orphaned,
            groups_skipped_too_few_tris,
            groups_skipped_target_too_small,
            groups_skipped_no_simplification,
            groups_skipped_clusterize_overflow,
            current_error,
            "LOD chain build: level done"
        );

        if !any_group_emitted_parent {
            tracing::info!(
                target: "ome_render::meshlet::builder",
                level = parent_lod_level,
                "LOD chain build: terminating — no group emitted parents"
            );
            break; // No progress this level; chain terminates.
        }

        prev_lod_indices = next_prev_lod_indices;
        current_error *= 2.0;
    }

    let total_roots = all_descriptors
        .iter()
        .filter(|m| m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT)
        .count();
    let max_lod_level = all_descriptors
        .iter()
        .map(|m| m.lod_level)
        .max()
        .unwrap_or(0);
    tracing::info!(
        target: "ome_render::meshlet::builder",
        total_meshlets = all_descriptors.len(),
        total_roots,
        max_lod_level,
        max_levels = lod_config.max_levels,
        "LOD chain build: done"
    );

    Ok(MeshletMesh {
        vertices: mesh.vertices.clone(),
        meshlet_vertices: all_meshlet_vertices,
        meshlet_triangles: all_meshlet_triangles,
        meshlets: all_descriptors,
        aabb: mesh_aabb,
    })
}

/// Geometry of a single meshlet group, prepared for
/// `meshopt::simplify_with_locks`. Vertex deduplication keeps the
/// simplifier's adapter efficient and the local index space compact.
struct GroupGeometry {
    /// Vertex stream in group-local order.
    vertices: Vec<MeshVertex>,
    /// Triangle indices into `vertices` (group-local).
    indices: Vec<u32>,
    /// `global_indices[i]` is the index into the source mesh's vertex
    /// pool that group-local vertex `i` refers to. Used after
    /// `clusterize_lod` to remap the resulting `meshlet_vertices`
    /// values back to the mesh-global pool, and to build the per-group
    /// vertex lock mask.
    global_indices: Vec<u32>,
}

/// Walks the meshlets in `group`, resolving every triangle through
/// `meshlet_triangles` (local byte indices) → `meshlet_vertices`
/// (global pool indices) → `mesh_vertices` (the actual MeshVertex).
/// Builds a deduplicated group-local vertex stream + triangle list.
fn extract_group_geometry(
    group: &[usize],
    prev_lod_meshlets: &[MeshletDescriptor],
    pool_meshlet_vertices: &[u32],
    pool_meshlet_triangles: &[u8],
    mesh_vertices: &[MeshVertex],
) -> GroupGeometry {
    let mut global_to_group: HashMap<u32, u32> = HashMap::new();
    let mut global_indices: Vec<u32> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for &meshlet_idx in group {
        let m = &prev_lod_meshlets[meshlet_idx];
        let tri_byte_start = m.triangle_offset as usize;
        let mlv_base = m.vertex_offset as usize;
        for tri_idx in 0..m.triangle_count as usize {
            for corner in 0..3usize {
                let byte_off = tri_byte_start + tri_idx * 3 + corner;
                let local_v_idx = pool_meshlet_triangles[byte_off] as usize;
                let global_v_idx = pool_meshlet_vertices[mlv_base + local_v_idx];
                let group_local = *global_to_group.entry(global_v_idx).or_insert_with(|| {
                    let new_idx = global_indices.len() as u32;
                    global_indices.push(global_v_idx);
                    new_idx
                });
                indices.push(group_local);
            }
        }
    }

    let vertices: Vec<MeshVertex> = global_indices
        .iter()
        .map(|&gi| mesh_vertices[gi as usize])
        .collect();

    GroupGeometry {
        vertices,
        indices,
        global_indices,
    }
}

#[cfg(test)]
mod tests;
