use super::*;
use crate::meshlet::asset::MESHLET_ROOT_PARENT;
use crate::meshlet::builder::test_support::{make_grid_mesh, vertex};

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

#[test]
fn single_lod_meshes_keep_root_sentinel_and_zero_error() {
    // Default builder is unchanged: every meshlet a root, error 0.
    let mesh = make_grid_mesh(20);
    let single = build_default_meshlets(&mesh).expect("build");
    for m in &single.meshlets {
        assert_eq!(m.parent_meshlet_index, MESHLET_ROOT_PARENT);
        assert_eq!(m.lod_error, 0.0);
    }
}

/// Every meshlet triangle still faces the way its mesh did.
///
/// 🔴 The layer the shadow raster actually reads, and the one the
/// primitives' own `assert_outward_facing` cannot reach. That test walks
/// `mesh.indices`; this walks `meshlet_triangles`, three bytes a
/// triangle, through `meshlet_vertices` — the same double indirection
/// `page_depth.wgsl` does. A `meshopt` pass that reordered a corner
/// would be invisible to the source test and would show up on screen as
/// a shadow with holes in it, which is the shape of a bug that is
/// expensive to chase from the wrong end.
#[test]
fn a_meshlet_triangle_faces_the_way_its_mesh_did() {
    use crate::mesh::primitives::Primitive;
    use glam::Vec3;

    for (name, primitive) in Primitive::CANONICAL {
        let mesh = primitive.build();
        let built = build_default_meshlets(&mesh).expect("the primitive builds meshlets");
        let mut checked = 0usize;
        for (m, desc) in built.meshlets.iter().enumerate() {
            for t in 0..desc.triangle_count as usize {
                let corner = |k: usize| {
                    let byte = desc.triangle_offset as usize + t * 3 + k;
                    let local = built.meshlet_triangles[byte] as usize;
                    let global = built.meshlet_vertices[desc.vertex_offset as usize + local];
                    built.vertices[global as usize]
                };
                let (a, b, c) = (corner(0), corner(1), corner(2));
                let pos = |v: &crate::mesh::MeshVertex| Vec3::from_array(v.position);
                let geometric = (pos(&b) - pos(&a)).cross(pos(&c) - pos(&a));
                if geometric.length() < 1e-6 {
                    continue;
                }
                let shading = Vec3::from_array(a.normal)
                    + Vec3::from_array(b.normal)
                    + Vec3::from_array(c.normal);
                assert!(
                    geometric.normalize().dot(shading.normalize_or_zero()) > 0.0,
                    "{name}: meshlet {m} triangle {t} winds inward — \
                     geometric {geometric:?} against shading {shading:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "{name} produced no triangles to check");
    }
}
