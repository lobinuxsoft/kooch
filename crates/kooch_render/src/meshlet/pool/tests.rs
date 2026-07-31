use crate::mesh::{Mesh, MeshVertex};
use crate::meshlet::build_default_meshlets;

use super::descriptor::MeshDescriptor;
use super::pool::GlobalMeshPool;

fn cube_mesh() -> Mesh {
    let positions = [
        [-0.5, -0.5, -0.5],
        [0.5, -0.5, -0.5],
        [0.5, 0.5, -0.5],
        [-0.5, 0.5, -0.5],
        [-0.5, -0.5, 0.5],
        [0.5, -0.5, 0.5],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, 0.5],
    ];
    let face_normals = [
        [0.0, 0.0, -1.0],
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
        [0.0, 1.0, 0.0],
        [-1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
    ];
    let face_indices: [[usize; 4]; 6] = [
        [0, 1, 2, 3],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [3, 2, 6, 7],
        [0, 3, 7, 4],
        [1, 2, 6, 5],
    ];
    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (face_idx, corners) in face_indices.iter().enumerate() {
        let normal = face_normals[face_idx];
        let base = vertices.len() as u32;
        for &c in corners {
            vertices.push(MeshVertex {
                position: positions[c],
                normal,
                uv: [0.0, 0.0],
            });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::from_arrays(vertices, indices)
}

#[test]
fn descriptor_layout_is_pod_32_bytes() {
    assert_eq!(MeshDescriptor::SIZE, 32);
}

#[test]
fn empty_pool_has_zero_meshes() {
    let pool = GlobalMeshPool::new();
    assert_eq!(pool.mesh_count(), 0);
    assert_eq!(pool.max_meshlets_per_mesh(), 0);
}

#[test]
fn register_increments_mesh_id() {
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");
    let mut pool = GlobalMeshPool::new();
    let h0 = pool.register(&mesh);
    let h1 = pool.register(&mesh);
    assert_eq!(h0.mesh_id, 0);
    assert_eq!(h1.mesh_id, 1);
    assert_eq!(pool.mesh_count(), 2);
}

#[test]
fn register_concatenates_arrays_with_correct_offsets() {
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");
    let mut pool = GlobalMeshPool::new();
    let h0 = pool.register(&mesh);
    let descriptor_0 = pool.mesh_descriptors[h0.mesh_id as usize];
    assert_eq!(descriptor_0.vertex_offset, 0);
    assert_eq!(descriptor_0.first_meshlet, 0);

    let len_v_0 = pool.vertices.len() as u32;
    let len_mv_0 = pool.meshlet_vertices.len() as u32;
    let len_meshlets_0 = pool.meshlets.len() as u32;

    let h1 = pool.register(&mesh);
    let descriptor_1 = pool.mesh_descriptors[h1.mesh_id as usize];
    assert_eq!(descriptor_1.vertex_offset, len_v_0);
    assert_eq!(descriptor_1.meshlet_vertex_offset, len_mv_0);
    assert_eq!(descriptor_1.first_meshlet, len_meshlets_0);
}

#[test]
fn meshlet_offsets_are_rebased_into_pool_coordinates() {
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");
    let mut pool = GlobalMeshPool::new();
    pool.register(&mesh);
    let len_mv = pool.meshlet_vertices.len() as u32;
    let len_mt = pool.meshlet_triangles.len() as u32;
    pool.register(&mesh);
    let second_slice_start = pool.meshlets.len() / 2;
    let mp = pool.meshlets[second_slice_start];
    let original = mesh.meshlets[0];
    assert_eq!(mp.vertex_offset, original.vertex_offset + len_mv);
    assert_eq!(mp.triangle_offset, original.triangle_offset + len_mt);
}

#[test]
fn parent_meshlet_index_values_are_rebased_into_pool_meshlet_space() {
    // Regression: `parent_meshlet_index` is a meshlet index into
    // the SAME chain. When the pool concatenates a second mesh,
    // its parents land inside the second mesh's slice — the
    // values must be shifted by the first_meshlet offset so the
    // 2-pass cull's pass 1 reads the correct parent's pixel
    // error. Before the fix, mesh #2's parents silently
    // referenced mesh #1's meshlets and the LOD selector
    // behaved randomly per mesh.
    use crate::meshlet::asset::MESHLET_ROOT_PARENT;
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");
    let mut pool = GlobalMeshPool::new();
    pool.register(&mesh);
    let first_mesh_meshlet_count = pool.meshlets.len() as u32;
    pool.register(&mesh);

    // Every parent index in the second registration's slice must
    // be either the root sentinel or land in the second mesh's
    // own range [first_mesh_meshlet_count, total).
    for m in &pool.meshlets[first_mesh_meshlet_count as usize..] {
        if m.parent_meshlet_index == MESHLET_ROOT_PARENT {
            continue;
        }
        assert!(
            m.parent_meshlet_index >= first_mesh_meshlet_count,
            "parent index {} in second mesh's slice must be >= {}",
            m.parent_meshlet_index,
            first_mesh_meshlet_count,
        );
        assert!(
            (m.parent_meshlet_index as usize) < pool.meshlets.len(),
            "parent index {} must stay within the pool",
            m.parent_meshlet_index,
        );
    }
}

#[test]
fn meshlet_vertices_values_are_rebased_into_pool_vertex_space() {
    // Regression: the pool used to extend_from_slice(meshlet_vertices)
    // verbatim, which left the second mesh's local indices pointing
    // back at the first mesh's vertices in the concatenated pool.
    // The shader's vertices[meshlet_vertices[..]] lookup then
    // produced random geometry. The fix shifts each value by the
    // mesh's vertex base offset on append.
    let mesh = build_default_meshlets(&cube_mesh()).expect("build");
    let mut pool = GlobalMeshPool::new();
    pool.register(&mesh);
    let first_mesh_vertex_count = pool.vertices.len() as u32;
    let first_mesh_meshlet_vertex_count = pool.meshlet_vertices.len();
    pool.register(&mesh);

    // Every value the second registration appended must reach into
    // the pool slice [first_mesh_vertex_count, 2 * first_mesh_vertex_count).
    for &v in &pool.meshlet_vertices[first_mesh_meshlet_vertex_count..] {
        assert!(
            v >= first_mesh_vertex_count,
            "meshlet_vertices value {v} from second mesh must point past the first mesh's slice (>= {first_mesh_vertex_count})",
        );
        assert!(
            (v as usize) < pool.vertices.len(),
            "meshlet_vertices value {v} must stay within the pool's vertex range ({})",
            pool.vertices.len(),
        );
    }
}
