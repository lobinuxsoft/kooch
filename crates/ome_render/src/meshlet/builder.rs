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

use crate::mesh::{Aabb, Mesh};

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

/// Builds a multi-LOD [`MeshletMesh`] by repeatedly simplifying the
/// source mesh and clusterising each level. The resulting `MeshletMesh`
/// concatenates every LOD's meshlets into one descriptor array,
/// rebasing per-LOD `vertex_offset` / `triangle_offset` so the GPU can
/// index a single flat buffer.
///
/// Each meshlet's `lod_error` carries the actual simplify error
/// `meshopt::simplify` reported for that LOD. `parent_meshlet_index`
/// stays at [`MESHLET_ROOT_PARENT`] until #442 sub-commit 3 wires the
/// DAG parent-child links.
///
/// LOD 0 is always present (the original mesh, error 0.0). Subsequent
/// levels are added as long as `meshopt::simplify` can keep reducing
/// the index count; the chain stops naturally on topology-bound meshes.
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

    let vertex_stride = std::mem::size_of::<crate::mesh::MeshVertex>();
    let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let adapter = meshopt::VertexDataAdapter::new(vertex_bytes, vertex_stride, 0)?;

    // LOD 0 — full detail.
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

    let mut current_indices = mesh.indices.clone();
    let mut current_error = lod_config.initial_error;

    for _level in 1..lod_config.max_levels {
        // Triangle counts must stay multiples of 3.
        let target_count = ((current_indices.len() as f32) * lod_config.target_ratio) as usize;
        let target_count = (target_count / 3) * 3;
        if target_count < 3 {
            break;
        }

        let mut actual_error = 0.0f32;
        let simplified = meshopt::simplify(
            &current_indices,
            &adapter,
            target_count,
            current_error,
            meshopt::SimplifyOptions::None,
            Some(&mut actual_error),
        );

        // simplify returns the original list when it can't reduce
        // further (topology lock, error budget exhausted, etc.).
        if simplified.is_empty() || simplified.len() >= current_indices.len() {
            break;
        }

        let (lod_descriptors, lod_meshlet_vertices, lod_meshlet_triangles) = clusterize_lod(
            &simplified,
            &adapter,
            &mesh.vertices,
            max_vertices,
            max_triangles,
            cone_weight,
            actual_error,
        );

        // Pad to 4-byte boundary before appending: the cull shader
        // reads array<u32> and extracts triangle bytes via shift+mask;
        // unaligned LOD-N base offsets would mis-read the first byte.
        while all_meshlet_triangles.len() % 4 != 0 {
            all_meshlet_triangles.push(0);
        }
        let vertex_offset_base = all_meshlet_vertices.len() as u32;
        let triangle_offset_base = all_meshlet_triangles.len() as u32;

        for desc in lod_descriptors {
            all_descriptors.push(MeshletDescriptor {
                vertex_offset: desc.vertex_offset + vertex_offset_base,
                triangle_offset: desc.triangle_offset + triangle_offset_base,
                ..desc
            });
        }
        all_meshlet_vertices.extend_from_slice(&lod_meshlet_vertices);
        all_meshlet_triangles.extend_from_slice(&lod_meshlet_triangles);

        current_indices = simplified;
        current_error *= 2.0;
    }

    assign_dag_parents(&mut all_descriptors);

    Ok(MeshletMesh {
        vertices: mesh.vertices.clone(),
        meshlet_vertices: all_meshlet_vertices,
        meshlet_triangles: all_meshlet_triangles,
        meshlets: all_descriptors,
        aabb: total_aabb(&mesh.vertices),
    })
}

/// Returns `[start, end)` ranges in `meshlets` corresponding to each
/// LOD level. Relies on `lod_error` being monotonically non-decreasing
/// across the concatenated chain — a property
/// [`build_meshlets_lod_chain`] guarantees by construction.
fn lod_level_ranges(meshlets: &[MeshletDescriptor]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    if meshlets.is_empty() {
        return ranges;
    }
    let mut start = 0;
    let mut current_error = meshlets[0].lod_error;
    for (i, m) in meshlets.iter().enumerate().skip(1) {
        if m.lod_error != current_error {
            ranges.push((start, i));
            start = i;
            current_error = m.lod_error;
        }
    }
    ranges.push((start, meshlets.len()));
    ranges
}

/// Wires each meshlet's `parent_meshlet_index` to the closest meshlet
/// in the next-coarser LOD by `bounds_center` distance. Meshlets in
/// the coarsest LOD keep [`MESHLET_ROOT_PARENT`].
///
/// **V1 — proximity only.** Nanite's reference algorithm groups four
/// children at LOD N and re-simplifies that group into a single LOD
/// N+1 parent (guarantees zero popping). That requires re-structuring
/// the chain build to operate per-cluster instead of globally; tracked
/// for a follow-up if profiling shows visible popping with this
/// approximation.
fn assign_dag_parents(meshlets: &mut [MeshletDescriptor]) {
    let ranges = lod_level_ranges(meshlets);
    if ranges.len() < 2 {
        return;
    }
    for level_idx in 0..ranges.len() - 1 {
        let (child_start, child_end) = ranges[level_idx];
        let (parent_start, parent_end) = ranges[level_idx + 1];
        for i in child_start..child_end {
            let cc = meshlets[i].bounds_center;
            let mut best: Option<(f32, usize)> = None;
            for j in parent_start..parent_end {
                let pc = meshlets[j].bounds_center;
                let dx = pc[0] - cc[0];
                let dy = pc[1] - cc[1];
                let dz = pc[2] - cc[2];
                let d2 = dx * dx + dy * dy + dz * dz;
                if best.map_or(true, |(b, _)| d2 < b) {
                    best = Some((d2, j));
                }
            }
            if let Some((_, parent_idx)) = best {
                meshlets[i].parent_meshlet_index = parent_idx as u32;
            }
        }
    }
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
    fn lod_chain_errors_are_monotonically_non_decreasing() {
        let mesh = make_grid_mesh(20);
        let chain = build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            LodConfig::default(),
        )
        .expect("lod chain");
        let mut prev = -1.0f32;
        for m in &chain.meshlets {
            assert!(
                m.lod_error >= prev,
                "lod_error must be non-decreasing across the concatenated chain; saw {} after {}",
                m.lod_error,
                prev
            );
            prev = m.lod_error;
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
    fn lod_chain_dag_only_coarsest_level_is_root() {
        let mesh = make_grid_mesh(20);
        let chain = build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            LodConfig::default(),
        )
        .expect("chain");
        let coarsest = chain
            .meshlets
            .iter()
            .map(|m| m.lod_error)
            .fold(f32::NEG_INFINITY, f32::max);
        for m in &chain.meshlets {
            let expect_root = m.lod_error == coarsest;
            let is_root =
                m.parent_meshlet_index == crate::meshlet::asset::MESHLET_ROOT_PARENT;
            assert_eq!(
                is_root, expect_root,
                "meshlet at lod_error={} should be root={} but is_root={}",
                m.lod_error, expect_root, is_root,
            );
        }
    }

    #[test]
    fn lod_chain_dag_parent_points_to_coarser_meshlet() {
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
            let p = &chain.meshlets[m.parent_meshlet_index as usize];
            assert!(
                p.lod_error > m.lod_error,
                "child #{i} (err {}) parent #{} must be coarser, got err {}",
                m.lod_error,
                m.parent_meshlet_index,
                p.lod_error,
            );
        }
    }

    #[test]
    fn lod_chain_dag_is_acyclic_via_descent_terminates() {
        // Walk from each meshlet up to a root following parent links;
        // assert termination within the LOD count (guards against
        // accidental cycles).
        let mesh = make_grid_mesh(20);
        let chain = build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            LodConfig::default(),
        )
        .expect("chain");
        let mut distinct_errors: Vec<f32> = chain.meshlets.iter().map(|m| m.lod_error).collect();
        distinct_errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
        distinct_errors.dedup();
        let max_steps = distinct_errors.len() + 1;
        for (i, m) in chain.meshlets.iter().enumerate() {
            let mut idx = i;
            for _ in 0..max_steps {
                let parent = chain.meshlets[idx].parent_meshlet_index;
                if parent == crate::meshlet::asset::MESHLET_ROOT_PARENT {
                    break;
                }
                idx = parent as usize;
            }
            // We must have hit a root within max_steps.
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
        let mesh = make_grid_mesh(40);
        let cfg = LodConfig {
            max_levels: 2,
            ..Default::default()
        };
        let chain = build_meshlets_lod_chain(
            &mesh,
            DEFAULT_MAX_VERTICES,
            DEFAULT_MAX_TRIANGLES,
            0.5,
            cfg,
        )
        .expect("chain");
        // With max_levels=2 we have at most 2 distinct lod_error values.
        let mut errors: Vec<f32> = chain.meshlets.iter().map(|m| m.lod_error).collect();
        errors.sort_by(|a, b| a.partial_cmp(b).unwrap());
        errors.dedup();
        assert!(errors.len() <= 2, "expected ≤2 distinct LODs, got {}", errors.len());
    }
}
