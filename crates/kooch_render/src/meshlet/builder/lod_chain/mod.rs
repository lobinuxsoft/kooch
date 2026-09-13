//! Multi-LOD Nanite-grouped DAG chain build. Concatenates every LOD's meshlets into one descriptor
//! array, rebasing per-LOD `vertex_offset` / `triangle_offset` so the GPU can index a single flat
//! buffer.

use std::collections::HashMap;

use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::asset::MeshletDescriptor;
use crate::meshlet::asset::MeshletMesh;

use super::common::{clusterize_lod, total_aabb};
use super::error::MeshletBuildError;
use super::grouping::{collect_group_boundary_vertices, group_meshlets_metis};
use super::lod_config::LodConfig;

/// Number of children grouped together when building each LOD level of the Nanite-style DAG. Karis
/// SIGGRAPH 2021 used 4 — small enough that simplify reliably collapses the group to a parent
/// meshlet (≤ MAX_TRIANGLES), large enough to amortise the per-group overhead.
pub(super) const NANITE_GROUP_SIZE: usize = 4;

/// Maximum parent meshlets a single group may emit (#535).
const PARENTS_PER_GROUP_CAP: usize = NANITE_GROUP_SIZE * 2;

/// LOD level past which per-vertex cell-boundary locks are dropped (#535).
const CELL_BOUNDARY_LOCK_MAX_LEVEL: u32 = 3;

/// Builds a multi-LOD [`MeshletMesh`] using the Nanite-grouped DAG algorithm. The resulting
/// `MeshletMesh` concatenates every LOD's meshlets into one descriptor array, rebasing per-LOD
/// `vertex_offset` / `triangle_offset` so the GPU can index a single flat buffer.
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
    let (mut all_descriptors, mut all_meshlet_vertices, mut all_meshlet_triangles) = clusterize_lod(
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
        target: "kooch_render::meshlet::builder",
        lod_zero_count,
        "LOD chain build: starting"
    );
    // Per-level input is a list of pool indices, not a contiguous range. The contiguous form
    // (prev_start..prev_end) assumed every member of the previous level got either a parent or
    // stayed rooted forever.
    let mut prev_lod_indices: Vec<u32> = (0..lod_zero_count as u32).collect();
    let mut current_error = lod_config.initial_error;
    // Sequential id assigned per group as the chain is built.
    let mut next_group_id: u32 = 0;

    for level in 1..lod_config.max_levels {
        // The chain depth for parents emitted in this iteration. LOD 0 sits at level 0; parents
        // from the first simplify are level 1; etc. The LOD-stack inspector (#467) and
        // MeshInstance.lod_force_level filter by this value.
        let parent_lod_level = level as u32;
        let prev_count = prev_lod_indices.len();
        if prev_count <= 1 {
            break; // Single meshlet at the previous level → cannot group.
        }

        // Snapshot the previous level's meshlets so the group loop can read them by local index
        // while parents get appended to the same all_descriptors vector. `prev_lod_indices[i]` maps
        // a group-local meshlet position back to its pool index.
        let prev_meshlets: Vec<MeshletDescriptor> = prev_lod_indices
            .iter()
            .map(|&pool_idx| all_descriptors[pool_idx as usize])
            .collect();

        let target_groups = (prev_count / NANITE_GROUP_SIZE).max(1);
        let groups = group_meshlets_metis(&prev_meshlets, &all_meshlet_vertices, target_groups);
        let group_boundary_globals =
            collect_group_boundary_vertices(&groups, &prev_meshlets, &all_meshlet_vertices);

        // Accumulator for the *next* level's input. Receives: - new parents emitted in this level
        // (the happy path) - orphaned children whose group failed to simplify (Plan C re-injection
        // — they get another shot when METIS groups them with neighbours next level)
        let mut next_prev_lod_indices: Vec<u32> = Vec::with_capacity(prev_count);
        let mut any_group_emitted_parent = false;
        // Per-level instrumentation (#535). Counts the funnel from METIS partitioning down to
        // actually-emitted parents so the root explosion can be diagnosed without re-running with a
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
            // Build the lock mask paralleling `geo.vertices`.
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
                // No reduction possible for this group. Plan C: re- inject the children into the
                // next level's input so METIS re-partitions them with neighbours that *did*
                // simplify.
                groups_skipped_no_simplification += 1;
                children_orphaned += group.len() as u32;
                for &child_local in group {
                    next_prev_lod_indices.push(prev_lod_indices[child_local]);
                }
                continue;
            }

            // Monotone-clamp the parent's lod_error to be ≥ every child's lod_error (#535 H1).
            // meshopt::simplify reports the *local* simplification error for this one step.
            let mut child_max_lod_error = 0.0f32;
            for &child_local in group {
                child_max_lod_error = child_max_lod_error.max(prev_meshlets[child_local].lod_error);
            }
            let parent_lod_error = actual_error.max(child_max_lod_error);

            let (parent_descs, parent_mlv_local, parent_mlt) = clusterize_lod(
                &simplified,
                &group_adapter,
                &geo.vertices,
                max_vertices,
                max_triangles,
                cone_weight,
                parent_lod_error,
            );
            if parent_descs.is_empty() || parent_descs.len() > PARENTS_PER_GROUP_CAP {
                // Empty: simplify produced nothing usable. Cap exceeded: even the relaxed bound of
                // 2× NANITE_GROUP_SIZE rejected — the geometry genuinely exploded into too many
                // small clusters for this group to count as progress.
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

            // Wire each child to the parent meshlet (within this group's emitted set) whose
            // `bounds_center` is closest to the child's.
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
            target: "kooch_render::meshlet::builder",
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
                target: "kooch_render::meshlet::builder",
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
        target: "kooch_render::meshlet::builder",
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
    /// `global_indices[i]` is the index into the source mesh's vertex pool that group-local vertex
    /// `i` refers to. Used after `clusterize_lod` to remap the resulting `meshlet_vertices` values
    /// back to the mesh-global pool, and to build the per-group vertex lock mask.
    global_indices: Vec<u32>,
}

/// Walks the meshlets in `group`, resolving every triangle through `meshlet_triangles` (local byte
/// indices) → `meshlet_vertices` (global pool indices) → `mesh_vertices` (the actual MeshVertex).
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
