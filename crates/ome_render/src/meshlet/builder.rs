//! `Mesh` → `MeshletMesh` offline builder using `meshopt`.
//!
//! Two production entry points:
//!
//! - [`build_meshlets_from_mesh`] — single-LOD output (the legacy /
//!   default path). Every meshlet is a DAG root with `lod_error = 0`.
//! - [`build_meshlets_lod_chain`] — runs `meshopt::simplify` repeatedly
//!   to produce a chain of LODs and concatenates them into one
//!   [`MeshletMesh`]. Each meshlet's `lod_error` carries the simplify
//!   error in mesh units; `parent_meshlet_index` is left at the root
//!   sentinel (the DAG parent-child links land in #442 sub-commit 3).

use glam::Vec3;

use crate::mesh::{Aabb, Mesh, MeshVertex};

use super::asset::{
    MeshletDescriptor, MeshletMesh, DEFAULT_MAX_TRIANGLES, DEFAULT_MAX_VERTICES,
    MESHLET_ROOT_PARENT,
};

/// Errors raised while building a meshlet mesh.
#[derive(Debug)]
pub enum MeshletBuildError {
    /// Source mesh had no triangles.
    EmptyMesh,
    /// `meshopt` rejected the vertex layout (stride mismatch, etc.).
    VertexAdapter(meshopt::Error),
}

impl std::fmt::Display for MeshletBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyMesh => write!(f, "cannot build meshlets from a mesh with zero triangles"),
            Self::VertexAdapter(e) => write!(f, "meshopt vertex adapter failed: {e}"),
        }
    }
}

impl std::error::Error for MeshletBuildError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::VertexAdapter(e) => Some(e),
            _ => None,
        }
    }
}

impl From<meshopt::Error> for MeshletBuildError {
    fn from(e: meshopt::Error) -> Self {
        Self::VertexAdapter(e)
    }
}

/// Configuration for [`build_meshlets_lod_chain`].
#[derive(Debug, Clone, Copy)]
pub struct LodConfig {
    /// Maximum number of LOD levels to attempt past LOD 0. The chain
    /// stops early when `meshopt::simplify` cannot reduce the index
    /// count further (typically when the topology is too constrained
    /// to simplify any more). Default: 6.
    pub max_levels: usize,
    /// Initial simplify error tolerance in mesh units. Doubles each
    /// level; balanced default: 0.01.
    pub initial_error: f32,
    /// Target ratio for index reduction per level. 0.5 halves the
    /// triangle count each step; 0.7 is gentler.
    pub target_ratio: f32,
}

impl Default for LodConfig {
    fn default() -> Self {
        Self {
            max_levels: 6,
            initial_error: 0.01,
            target_ratio: 0.5,
        }
    }
}

/// Builds a [`MeshletMesh`] from `mesh` using `meshopt`'s clusterization.
///
/// Single-LOD output: every meshlet is a DAG root with `lod_error = 0`.
/// Use [`build_meshlets_lod_chain`] to produce a multi-LOD asset.
///
/// `max_vertices` and `max_triangles` cap each meshlet's size — the
/// defaults ([`DEFAULT_MAX_VERTICES`] / [`DEFAULT_MAX_TRIANGLES`]) match
/// every mesh-shader-capable GPU's recommended limits.
///
/// `cone_weight` weights the spatial vs. cone-tightness cost during
/// clusterisation. `0.5` is a balanced default; `0.0` ignores cones
/// (meshlets cluster by spatial proximity only); `1.0` heavily favours
/// tight cones at the cost of spatial coherence.
pub fn build_meshlets_from_mesh(
    mesh: &Mesh,
    max_vertices: usize,
    max_triangles: usize,
    cone_weight: f32,
) -> Result<MeshletMesh, MeshletBuildError> {
    if mesh.indices.is_empty() {
        return Err(MeshletBuildError::EmptyMesh);
    }

    let vertex_stride = std::mem::size_of::<crate::mesh::MeshVertex>();
    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let adapter = meshopt::VertexDataAdapter::new(vertex_bytes, vertex_stride, 0)?;

    let (descriptors, meshlet_vertices, meshlet_triangles) = clusterize_lod(
        &mesh.indices,
        &adapter,
        &mesh.vertices,
        max_vertices,
        max_triangles,
        cone_weight,
        0.0,
    );

    Ok(MeshletMesh {
        vertices: mesh.vertices.clone(),
        meshlet_vertices,
        meshlet_triangles,
        meshlets: descriptors,
        aabb: total_aabb(&mesh.vertices),
    })
}

/// Number of children grouped together when building each LOD level
/// of the Nanite-style DAG. Karis SIGGRAPH 2021 used 4 — small enough
/// that simplify reliably collapses the group to a single parent
/// meshlet (≤ MAX_TRIANGLES), large enough to amortise the per-group
/// overhead.
const NANITE_GROUP_SIZE: usize = 4;

/// Builds a multi-LOD [`MeshletMesh`] using the Nanite-grouped DAG
/// algorithm. The resulting `MeshletMesh` concatenates every LOD's
/// meshlets into one descriptor array, rebasing per-LOD
/// `vertex_offset` / `triangle_offset` so the GPU can index a single
/// flat buffer.
///
/// # Algorithm (per LOD level)
///
/// 1. Take the previous LOD's meshlets.
/// 2. Spatial-sort them by Morton code on `bounds_center`.
/// 3. Chunk into groups of [`NANITE_GROUP_SIZE`] consecutive meshlets.
/// 4. For each group:
///    - Resolve the group's underlying triangles into a local
///      vertex-deduped sub-mesh.
///    - Run `meshopt::simplify` with `LockBorder` so external edges
///      stay intact and adjacent groups stitch seamlessly.
///    - Re-cluster the simplified triangles. Target count is forced
///      to [`max_triangles`] so the result is one parent meshlet per
///      group — guarantees coverage of the four children's region.
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

    let mut prev_lod_range = (0usize, all_descriptors.len());
    let mut current_error = lod_config.initial_error;
    // Sequential id assigned per group as the chain is built. Every
    // (children, parents) pair from the same simplification step
    // shares the same group_id — children store it as `group_index`
    // ("the group I'm a child of"), parents store it as
    // `children_group_index` ("the group I'm a parent of"). The
    // 2-pass cull (#465) keys group_max_err by this id.
    let mut next_group_id: u32 = 0;

    for level in 1..lod_config.max_levels {
        // The chain depth for parents emitted in this iteration —
        // LOD 0 sits at level 0; parents from the first simplify are
        // level 1; etc. The LOD-stack inspector (#467) and
        // MeshInstance.lod_force_level filter by this value.
        let parent_lod_level = level as u32;
        let (prev_start, prev_end) = prev_lod_range;
        let prev_count = prev_end - prev_start;
        if prev_count <= 1 {
            break; // Single meshlet at the previous level → cannot group.
        }

        // Snapshot the previous LOD's meshlets so we can index by
        // group while still appending parents to the same vector.
        let prev_meshlets: Vec<MeshletDescriptor> =
            all_descriptors[prev_start..prev_end].to_vec();

        // V-partition grouping (Ponchio §3.5). Target `prev_count /
        // NANITE_GROUP_SIZE` Voronoi cells so each group averages ~4
        // meshlets — same ratio as the Morton chunker but with
        // boundaries that come from the partition cells (consistent
        // across LODs by construction) instead of arbitrary
        // chunk-of-4 cuts.
        let target_groups = (prev_count / NANITE_GROUP_SIZE).max(1);
        let groups = group_meshlets_voronoi(&prev_meshlets, target_groups);

        let new_lod_start_in_pool = all_descriptors.len();
        let mut any_group_emitted_parent = false;

        for group in &groups {
            let geo = extract_group_geometry(
                group,
                &prev_meshlets,
                &all_meshlet_vertices,
                &all_meshlet_triangles,
                &mesh.vertices,
            );
            // Need at least two triangles to produce a meaningful
            // simplification budget; smaller groups stay as roots.
            if geo.indices.len() < 6 {
                continue;
            }

            let group_vertex_bytes: &[u8] = bytemuck::cast_slice(&geo.vertices);
            let group_adapter = meshopt::VertexDataAdapter::new(
                group_vertex_bytes,
                vertex_stride,
                0,
            )?;

            // Single-pass simplify: aim at half the indices, accept
            // whatever clusterize emits up to NANITE_GROUP_SIZE
            // parents. Forcing exactly one parent (earlier iteration)
            // skipped ~97 % of groups in practice — vertex-count
            // limits inside max_meshlet bounds defeat the
            // single-parent guarantee on dense surfaces. Children
            // get assigned to their nearest parent within the group;
            // coherence stays inside the group so the seam-tearing
            // case from the proximity DAG (children of group X
            // pointing to parents from group Y) cannot recur.
            let target_count = ((max_triangles * 3) as usize)
                .min(((geo.indices.len() as f32) * lod_config.target_ratio) as usize);
            let target_count = (target_count / 3) * 3;
            if target_count < 3 {
                continue;
            }

            let mut actual_error = 0.0f32;
            let simplified = meshopt::simplify(
                &geo.indices,
                &group_adapter,
                target_count,
                current_error,
                meshopt::SimplifyOptions::LockBorder,
                Some(&mut actual_error),
            );
            if simplified.is_empty() || simplified.len() >= geo.indices.len() {
                continue; // No reduction possible for this group.
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
            if parent_descs.is_empty() || parent_descs.len() > NANITE_GROUP_SIZE {
                // Empty: simplify produced nothing usable. Too many
                // parents (more than the group's child count): the
                // simplification didn't actually compact the
                // geometry meaningfully — skip rather than emit a
                // worse-than-input level.
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
                    // Parents share `children_group_index = group_id`
                    // ("the group I'm a parent of"). Pass 2 reads
                    // group_max_err[children_group_index] for the
                    // "below_fine" descent test. The parent's own
                    // `group_index` ("group I'm a child of") will be
                    // populated when this parent is processed as a
                    // child in the next-level transition; it stays at
                    // MESHLET_GROUP_NONE for now (root behaviour).
                    children_group_index: group_id,
                    // Chain depth for these parents — first iteration
                    // emits level 1, next 2, etc. clusterize_lod
                    // defaults to 0; we override here.
                    lod_level: parent_lod_level,
                    ..*desc
                });
            }
            all_meshlet_vertices.extend(remapped_mlv);
            all_meshlet_triangles.extend(parent_mlt);

            // Wire each child to the parent meshlet (within this
            // group's emitted set) whose `bounds_center` is closest
            // to the child's. Confines parent-child coherence to
            // the group, so the proximity-DAG tearing case (children
            // of group X pointing at parents from group Y) is
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
                let child = &mut all_descriptors[prev_start + child_local];
                child.parent_meshlet_index = best.1;
                child.group_index = group_id;
            }

            any_group_emitted_parent = true;
        }

        if !any_group_emitted_parent {
            break; // No progress this level; chain terminates.
        }

        prev_lod_range = (new_lod_start_in_pool, all_descriptors.len());
        current_error *= 2.0;
    }

    Ok(MeshletMesh {
        vertices: mesh.vertices.clone(),
        meshlet_vertices: all_meshlet_vertices,
        meshlet_triangles: all_meshlet_triangles,
        meshlets: all_descriptors,
        aabb: mesh_aabb,
    })
}

/// Geometry of a single meshlet group, prepared for `meshopt::simplify`.
/// Vertex deduplication keeps the simplifier's adapter efficient and
/// the local index space compact.
struct GroupGeometry {
    /// Vertex stream in group-local order.
    vertices: Vec<MeshVertex>,
    /// Triangle indices into `vertices` (group-local).
    indices: Vec<u32>,
    /// `global_indices[i]` is the index into the source mesh's vertex
    /// pool that group-local vertex `i` refers to. Used after
    /// `clusterize_lod` to remap the resulting `meshlet_vertices`
    /// values back to the mesh-global pool.
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
    use std::collections::HashMap;
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

/// Picks `k` seed positions from `points` using farthest-point
/// sampling: starts with `points[0]`, then repeatedly picks the
/// point furthest from any already-selected seed. Deterministic,
/// O(n × k), and produces well-spread seeds without an RNG.
///
/// The output drives the V-partition grouper (#469): each meshlet
/// joins the group whose seed is nearest its `bounds_center`.
/// Lloyd's relaxation refines the seed positions afterwards so
/// each Voronoi cell ends up roughly equal-area.
fn pick_initial_seeds_farthest(points: &[Vec3], k: usize) -> Vec<Vec3> {
    if points.is_empty() || k == 0 {
        return Vec::new();
    }
    let k = k.min(points.len());
    let mut seeds = Vec::with_capacity(k);
    seeds.push(points[0]);

    // For each input point, track its squared distance to the
    // nearest already-chosen seed. The next seed is whichever
    // point has the largest such distance.
    let mut min_dist_sq: Vec<f32> = points
        .iter()
        .map(|p| (*p - seeds[0]).length_squared())
        .collect();

    while seeds.len() < k {
        let (idx, _) = min_dist_sq
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        let new_seed = points[idx];
        seeds.push(new_seed);
        for (i, p) in points.iter().enumerate() {
            let d = (*p - new_seed).length_squared();
            if d < min_dist_sq[i] {
                min_dist_sq[i] = d;
            }
        }
    }
    seeds
}

/// Returns the index of the seed closest to `p`. Linear scan; for the
/// meshlet counts we work with (a few thousand per LOD) this is well
/// within budget for offline build time.
fn nearest_seed_index(p: Vec3, seeds: &[Vec3]) -> usize {
    let mut best = (f32::INFINITY, 0usize);
    for (i, s) in seeds.iter().enumerate() {
        let d = (p - *s).length_squared();
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// Runs `iterations` of Lloyd's relaxation: assign each point to its
/// nearest seed, move each seed to the barycenter of its assignment,
/// repeat. Empty cells (no assigned points) keep their previous seed
/// position so the seed count never collapses.
///
/// Converges fast — Ponchio §3.5.3 recommends 3-5 iterations for the
/// V-partition use case. After convergence the Voronoi cells
/// partition the mesh into roughly equal-area, compact patches.
fn lloyd_relaxation(points: &[Vec3], initial_seeds: Vec<Vec3>, iterations: usize) -> Vec<Vec3> {
    let mut seeds = initial_seeds;
    if seeds.is_empty() || points.is_empty() {
        return seeds;
    }
    let n = seeds.len();
    let mut sums = vec![Vec3::ZERO; n];
    let mut counts = vec![0u32; n];
    for _ in 0..iterations {
        sums.iter_mut().for_each(|s| *s = Vec3::ZERO);
        counts.iter_mut().for_each(|c| *c = 0);
        for &p in points {
            let cell = nearest_seed_index(p, &seeds);
            sums[cell] += p;
            counts[cell] += 1;
        }
        for i in 0..n {
            if counts[i] > 0 {
                seeds[i] = sums[i] / counts[i] as f32;
            }
            // else: keep previous seed position; an empty cell would
            // otherwise drift to (0,0,0) which biases the partition.
        }
    }
    seeds
}

/// Number of Lloyd's relaxation iterations applied to the V-partition
/// seed sets. Ponchio §3.5.3 recommends 3-5; converges quickly and
/// extra iterations don't move the seeds appreciably.
const LLOYD_ITERATIONS: usize = 5;

/// Voronoi V-partition grouping (Ponchio §3.5, #469). Picks
/// `target_groups` seeds via farthest-point sampling, refines them
/// with Lloyd's relaxation so each cell ends up roughly equal-area,
/// then assigns every meshlet to its nearest seed by `bounds_center`.
///
/// Replaces the Morton-chunk grouping (#465 V1) which spatial-sorted
/// meshlets and chopped them into 4-tuples regardless of cluster
/// shape — that produced groups with arbitrary boundaries and the
/// boundary-management problem (Ponchio §3.4.3) bit us on coarse
/// LODs (visible holes when sibling parents collapsed their shared
/// non-topological border differently).
fn group_meshlets_voronoi(
    meshlets: &[MeshletDescriptor],
    target_groups: usize,
) -> Vec<Vec<usize>> {
    if meshlets.is_empty() || target_groups == 0 {
        return Vec::new();
    }
    let centroids: Vec<Vec3> = meshlets
        .iter()
        .map(|m| Vec3::from_array(m.bounds_center))
        .collect();
    let initial = pick_initial_seeds_farthest(&centroids, target_groups);
    let seeds = lloyd_relaxation(&centroids, initial, LLOYD_ITERATIONS);

    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); seeds.len()];
    for (i, c) in centroids.iter().enumerate() {
        let cell = nearest_seed_index(*c, &seeds);
        groups[cell].push(i);
    }
    // Drop empty cells — they happen when Lloyd's leaves a seed
    // with no nearest point (rare for well-distributed inputs but
    // guarded against here).
    groups.retain(|g| !g.is_empty());
    groups
}

/// Runs `meshopt::build_meshlets` over `indices` and returns the
/// per-meshlet descriptors plus the per-LOD `meshlet_vertices` and
/// `meshlet_triangles` arrays. `lod_error` tags every descriptor with
/// the simplify error that produced this LOD level (0.0 for LOD 0).
///
/// `parent_meshlet_index` defaults to [`MESHLET_ROOT_PARENT`] — the
/// DAG construction in #442 sub-commit 3 overwrites it.
fn clusterize_lod(
    indices: &[u32],
    adapter: &meshopt::VertexDataAdapter<'_>,
    vertex_pool: &[crate::mesh::MeshVertex],
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
            // Group ids are wired during the LOD chain build; for
            // single-LOD output and freshly clusterised LOD 0 they
            // remain at the sentinel until the chain pass overwrites
            // them. lod_level defaults to 0 (LOD 0 / unsimplified)
            // — chain pass overwrites for parents emitted at deeper
            // levels.
            group_index: crate::meshlet::asset::MESHLET_GROUP_NONE,
            children_group_index: crate::meshlet::asset::MESHLET_GROUP_NONE,
            lod_level: 0,
            _pad4: 0,
            _pad5: 0,
        });
    }
    (descriptors, raw.vertices, raw.triangles)
}

fn total_aabb(vertices: &[crate::mesh::MeshVertex]) -> Aabb {
    let mut aabb = Aabb::empty();
    for v in vertices {
        aabb.expand(Vec3::from_array(v.position));
    }
    aabb
}

/// Per-meshlet AABB, computed from the meshlet's vertex slice. The
/// meshlet's `vertices` slice stores indices INTO the parent mesh's
/// vertex array, so we look up positions there directly.
fn meshlet_aabb(
    meshlet: meshopt::Meshlet<'_>,
    vertex_pool: &[crate::mesh::MeshVertex],
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

/// Convenience wrapper using the default meshlet sizing
/// ([`DEFAULT_MAX_VERTICES`] = 64 verts, [`DEFAULT_MAX_TRIANGLES`] = 124
/// tris) and a balanced `cone_weight` of `0.5`.
pub fn build_default_meshlets(mesh: &Mesh) -> Result<MeshletMesh, MeshletBuildError> {
    build_meshlets_from_mesh(
        mesh,
        DEFAULT_MAX_VERTICES,
        DEFAULT_MAX_TRIANGLES,
        0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::MeshVertex;

    fn vertex(p: [f32; 3]) -> MeshVertex {
        MeshVertex {
            position: p,
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
        }
    }

    #[test]
    fn empty_mesh_returns_empty_error() {
        let mesh = Mesh::empty();
        let err = build_default_meshlets(&mesh).unwrap_err();
        assert!(matches!(err, MeshletBuildError::EmptyMesh));
    }

    #[test]
    fn single_triangle_yields_one_meshlet() {
        let mesh = Mesh::from_arrays(
            vec![
                vertex([0.0, 0.0, 0.0]),
                vertex([1.0, 0.0, 0.0]),
                vertex([0.0, 1.0, 0.0]),
            ],
            vec![0, 1, 2],
        );

        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.meshlet_count(), 1);
        assert_eq!(meshlet_mesh.meshlets[0].triangle_count, 1);
        assert!(meshlet_mesh.meshlets[0].vertex_count >= 3);
    }

    #[test]
    fn quad_yields_meshlet_covering_two_triangles() {
        // Quad: 4 vertices, 2 triangles.
        let mesh = Mesh::from_arrays(
            vec![
                vertex([0.0, 0.0, 0.0]),
                vertex([1.0, 0.0, 0.0]),
                vertex([1.0, 1.0, 0.0]),
                vertex([0.0, 1.0, 0.0]),
            ],
            vec![0, 1, 2, 0, 2, 3],
        );

        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.total_triangle_count(), 2);
        // Bounding sphere should cover the quad's extent.
        let m = &meshlet_mesh.meshlets[0];
        assert!(m.bounding_radius > 0.5);
    }

    #[test]
    fn total_aabb_covers_every_vertex() {
        let mesh = Mesh::from_arrays(
            vec![
                vertex([-2.0, -3.0, 1.0]),
                vertex([5.0, 4.0, 6.0]),
                vertex([0.0, 0.0, 0.0]),
            ],
            vec![0, 1, 2],
        );

        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.aabb.min, glam::Vec3::new(-2.0, -3.0, 0.0));
        assert_eq!(meshlet_mesh.aabb.max, glam::Vec3::new(5.0, 4.0, 6.0));
    }

    #[test]
    fn vertices_are_copied_into_pool() {
        let mesh = Mesh::from_arrays(
            vec![
                vertex([0.0, 0.0, 0.0]),
                vertex([1.0, 0.0, 0.0]),
                vertex([0.0, 1.0, 0.0]),
            ],
            vec![0, 1, 2],
        );
        let meshlet_mesh = build_default_meshlets(&mesh).expect("build");
        assert_eq!(meshlet_mesh.total_vertex_count(), 3);
    }

    /// Builds a denser triangulated grid (`subdivisions × subdivisions`
    /// quads) so meshopt::simplify has room to reduce. Returns a mesh
    /// in the XY plane spanning `[0, 1]²`.
    fn make_grid_mesh(subdivisions: usize) -> Mesh {
        let n = subdivisions + 1;
        let mut verts = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                verts.push(vertex([
                    x as f32 / subdivisions as f32,
                    y as f32 / subdivisions as f32,
                    0.0,
                ]));
            }
        }
        let mut idx = Vec::with_capacity(subdivisions * subdivisions * 6);
        for y in 0..subdivisions {
            for x in 0..subdivisions {
                let a = (y * n + x) as u32;
                let b = a + 1;
                let c = a + n as u32;
                let d = c + 1;
                idx.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
        Mesh::from_arrays(verts, idx)
    }

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
        // Per-group simplify (Nanite-grouped DAG) gives every parent
        // its own lod_error reported by meshopt for that group, so the
        // global error sequence is no longer monotonic across the
        // concatenated chain. The structural invariant that survives:
        // every LOD 0 meshlet (error == 0.0) lands before any LOD ≥ 1
        // meshlet (error > 0.0) because LOD 0 is appended in one
        // global pass before the per-group loop runs.
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
        // Per-group DAG: the chain terminates when no group can
        // simplify further. Every meshlet that did not get a parent
        // assigned during the loop is left at MESHLET_ROOT_PARENT;
        // the chain must end with at least one such terminal node so
        // the runtime selector has somewhere to stop descending.
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
            .filter(|m| m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT)
            .count();
        assert!(
            root_count > 0,
            "chain must contain at least one root meshlet (parent sentinel)",
        );
    }

    #[test]
    fn lod_chain_dag_parents_point_into_chain_bounds() {
        // Every non-root parent_meshlet_index references a real
        // meshlet that lives later in the chain (parents are appended
        // after children).
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
            if m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT {
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
        // strictly after their children in the chain, so length is a
        // safe upper bound on the descent depth.
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
                if parent == crate::meshlet::asset::MESHLET_ROOT_PARENT {
                    break;
                }
                idx = parent as usize;
            }
            assert_eq!(
                chain.meshlets[idx].parent_meshlet_index,
                crate::meshlet::asset::MESHLET_ROOT_PARENT,
                "DAG descent from #{i} did not terminate within {max_steps} steps",
            );
        }
    }

    #[test]
    fn lod_chain_dag_group_ids_propagate_to_both_sides() {
        // For every group emitted during chain construction, the
        // group's children share `group_index = id` and the group's
        // parents share `children_group_index = id`. Validates the
        // 2-pass cull contract: every meshlet that points at a real
        // parent_meshlet_index also has a real group_index, and vice
        // versa for parent meshlets that own children below.
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
            // A meshlet that has a parent must also have a group
            // (the group whose parents include `m.parent_meshlet_index`).
            if m.parent_meshlet_index != crate::meshlet::asset::MESHLET_ROOT_PARENT {
                assert_ne!(
                    m.group_index,
                    crate::meshlet::asset::MESHLET_GROUP_NONE,
                    "meshlet with parent must have a non-NONE group_index",
                );
            }
        }
        // Every meshlet referenced as a parent_meshlet_index by some
        // child must carry a non-NONE children_group_index.
        for m in &chain.meshlets {
            if m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT {
                continue;
            }
            let parent = &chain.meshlets[m.parent_meshlet_index as usize];
            assert_ne!(
                parent.children_group_index,
                crate::meshlet::asset::MESHLET_GROUP_NONE,
                "any meshlet referenced as a parent must have a non-NONE children_group_index",
            );
        }
    }

    #[test]
    fn lod_chain_dag_group_siblings_share_group_id() {
        // All children pointing at parents with the same
        // children_group_index must themselves share the same
        // group_index. The 2-pass cull's atomicMax convergence
        // depends on this.
        let mesh = make_grid_mesh(20);
        let chain = build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            LodConfig::default(),
        )
        .expect("chain");
        use std::collections::HashMap;
        let mut group_of_children: HashMap<u32, u32> = HashMap::new();
        for m in &chain.meshlets {
            if m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT {
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
    fn single_lod_meshes_keep_root_sentinel_and_zero_error() {
        // Default builder is unchanged: every meshlet a root, error 0.
        let mesh = make_grid_mesh(20);
        let single = build_default_meshlets(&mesh).expect("build");
        for m in &single.meshlets {
            assert_eq!(
                m.parent_meshlet_index,
                crate::meshlet::asset::MESHLET_ROOT_PARENT
            );
            assert_eq!(m.lod_error, 0.0);
        }
    }

    #[test]
    fn farthest_point_seeds_are_well_spread() {
        // Inputs along a 1D line (x = 0..=9); farthest-point seeding
        // with k=4 picks the extremes first then fills the interior.
        // The exact interior picks depend on tie-breaking, so the
        // assertion focuses on what the algorithm guarantees:
        //   1. requested count of unique seeds returned;
        //   2. the diameter of the input is covered (extremes
        //      included);
        //   3. no two seeds are duplicated.
        let pts: Vec<Vec3> = (0..10).map(|i| Vec3::new(i as f32, 0.0, 0.0)).collect();
        let seeds = pick_initial_seeds_farthest(&pts, 4);
        assert_eq!(seeds.len(), 4);
        let mut xs: Vec<f32> = seeds.iter().map(|s| s.x).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(xs[0], 0.0, "smallest seed should be at x=0");
        assert_eq!(xs[3], 9.0, "largest seed should be at x=9");
        // No duplicates.
        for w in xs.windows(2) {
            assert_ne!(w[0], w[1], "seeds {:?} contain a duplicate", xs);
        }
        // Interior seeds lie strictly between the endpoints.
        assert!(xs[1] > 0.0 && xs[1] < 9.0);
        assert!(xs[2] > 0.0 && xs[2] < 9.0);
    }

    #[test]
    fn lloyd_relaxation_settles_seeds_toward_barycenters() {
        // Two clusters of 4 points each, 10 units apart on X. Two
        // seeds initialised badly (both near the left cluster) —
        // Lloyd's should pull one of them to the right cluster's
        // barycentre.
        let mut pts = Vec::new();
        for &x in &[-5.0_f32, -4.5, -4.0, -3.5] {
            pts.push(Vec3::new(x, 0.0, 0.0));
        }
        for &x in &[5.0_f32, 5.5, 6.0, 6.5] {
            pts.push(Vec3::new(x, 0.0, 0.0));
        }
        let initial = vec![Vec3::new(-5.0, 0.0, 0.0), Vec3::new(-3.5, 0.0, 0.0)];
        let seeds = lloyd_relaxation(&pts, initial, 10);
        // After relaxation, one seed should sit near each cluster's
        // barycentre (~-4.25 and ~5.75).
        let mut xs: Vec<f32> = seeds.iter().map(|s| s.x).collect();
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!(
            xs[0] < -3.0,
            "left seed should sit near -4.25, got {}",
            xs[0]
        );
        assert!(
            xs[1] > 4.0,
            "right seed should sit near 5.75, got {}",
            xs[1]
        );
    }

    #[test]
    fn voronoi_groups_are_spatially_coherent() {
        // Build a real chain and verify that meshlets in the same
        // group are spatially closer (mean pairwise distance) than
        // a random sample across the whole mesh. The grouping is
        // an internal invariant of the V-partition build — without
        // spatial coherence the per-group simplify falls apart.
        let mesh = make_grid_mesh(20);
        let chain = build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            LodConfig::default(),
        )
        .expect("chain");
        // Compare mean intra-group distance vs mean overall distance
        // among the LOD 0 meshlets (all share lod_error == 0).
        let lod_zero: Vec<&MeshletDescriptor> =
            chain.meshlets.iter().filter(|m| m.lod_error == 0.0).collect();
        if lod_zero.len() < 4 {
            return; // Not enough data for a meaningful comparison.
        }

        // Bucket LOD-0 meshlets by group_index (skip roots).
        use std::collections::HashMap;
        let mut by_group: HashMap<u32, Vec<Vec3>> = HashMap::new();
        for m in &lod_zero {
            if m.group_index == crate::meshlet::asset::MESHLET_GROUP_NONE {
                continue;
            }
            by_group
                .entry(m.group_index)
                .or_default()
                .push(Vec3::from_array(m.bounds_center));
        }
        if by_group.is_empty() {
            return;
        }

        let intra_mean = mean_pairwise_distance(by_group.values().flat_map(|g| {
            g.windows(2).map(|w| (w[0] - w[1]).length())
        }));
        let all_centroids: Vec<Vec3> =
            lod_zero.iter().map(|m| Vec3::from_array(m.bounds_center)).collect();
        let global_mean = mean_pairwise_distance(
            all_centroids.windows(2).map(|w| (w[0] - w[1]).length()),
        );

        assert!(
            intra_mean <= global_mean,
            "Voronoi groups should be spatially coherent: intra-group mean \
             distance {} must be ≤ global mean {}",
            intra_mean,
            global_mean,
        );
    }

    fn mean_pairwise_distance<I: Iterator<Item = f32>>(iter: I) -> f32 {
        let (sum, n) = iter.fold((0.0_f32, 0u32), |(s, c), d| (s + d, c + 1));
        if n == 0 { 0.0 } else { sum / n as f32 }
    }

    #[test]
    fn lod_chain_caps_at_max_levels() {
        // max_levels controls how many descent passes the per-group
        // loop runs. Compare two chains: a tighter cap must produce
        // ≤ the meshlet count of a looser cap. (We can't assert
        // distinct lod_error values any more — per-group simplify
        // makes each parent carry its own error.)
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
}
