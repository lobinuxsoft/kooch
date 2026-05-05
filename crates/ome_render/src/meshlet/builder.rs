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

    for _level in 1..lod_config.max_levels {
        let (prev_start, prev_end) = prev_lod_range;
        let prev_count = prev_end - prev_start;
        if prev_count <= 1 {
            break; // Single meshlet at the previous level → cannot group.
        }

        // Snapshot the previous LOD's meshlets so we can index by
        // group while still appending parents to the same vector.
        let prev_meshlets: Vec<MeshletDescriptor> =
            all_descriptors[prev_start..prev_end].to_vec();

        let groups =
            group_meshlets_morton(&prev_meshlets, &mesh_aabb, NANITE_GROUP_SIZE);

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

            let first_parent_pool_idx = all_descriptors.len() as u32;
            for desc in &parent_descs {
                all_descriptors.push(MeshletDescriptor {
                    vertex_offset: desc.vertex_offset + vertex_offset_base,
                    triangle_offset: desc.triangle_offset + triangle_offset_base,
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
                all_descriptors[prev_start + child_local].parent_meshlet_index = best.1;
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

/// Quantises `value in [min, max]` into a 21-bit integer for use as
/// one axis of a 3D Morton code.
fn quantize_axis_21bit(value: f32, min: f32, max: f32) -> u64 {
    let range = (max - min).max(1.0e-6);
    let normalised = ((value - min) / range).clamp(0.0, 1.0);
    let max_int = ((1u64 << 21) - 1) as f32;
    (normalised * max_int) as u64
}

/// Spreads the lower 21 bits of `v` so they occupy every third bit
/// position — the building block of a 3D Morton code.
fn spread_21bit(mut v: u64) -> u64 {
    v &= 0x1f_ffff;
    v = (v | v << 32) & 0x001f_0000_0000_ffff;
    v = (v | v << 16) & 0x001f_0000_ff00_00ff;
    v = (v | v << 8) & 0x100f_00f0_0f00_f00f;
    v = (v | v << 4) & 0x10c3_0c30_c30c_30c3;
    v = (v | v << 2) & 0x1249_2492_4924_9249;
    v
}

/// Interleaves three 21-bit axis values into a 63-bit Morton code.
fn morton3_21bit(x: u64, y: u64, z: u64) -> u64 {
    spread_21bit(x) | (spread_21bit(y) << 1) | (spread_21bit(z) << 2)
}

/// Spatial-sorts `meshlets` by 3D Morton code on `bounds_center`,
/// then chunks them into groups of `group_size` consecutive items.
/// Morton ordering keeps spatially close meshlets near each other in
/// the chunk sequence — the key property the per-group simplify needs
/// for coverage to make sense.
fn group_meshlets_morton(
    meshlets: &[MeshletDescriptor],
    aabb: &Aabb,
    group_size: usize,
) -> Vec<Vec<usize>> {
    let mut keyed: Vec<(u64, usize)> = meshlets
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let c = m.bounds_center;
            let x = quantize_axis_21bit(c[0], aabb.min.x, aabb.max.x);
            let y = quantize_axis_21bit(c[1], aabb.min.y, aabb.max.y);
            let z = quantize_axis_21bit(c[2], aabb.min.z, aabb.max.z);
            (morton3_21bit(x, y, z), i)
        })
        .collect();
    keyed.sort_unstable_by_key(|&(code, _)| code);
    keyed
        .chunks(group_size.max(1))
        .map(|chunk| chunk.iter().map(|&(_, i)| i).collect())
        .collect()
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
            _pad2: 0,
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
