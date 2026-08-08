use super::*;

#[test]
fn descriptor_layout_is_repr_c_pod() {
    // Fixed size lets the GPU upload assume a stride. History:
    //   80 B (PR initial)
    //   96 B (PR-5b: real cone_apex separate from bounding centre)
    //  112 B (#465: group_index + children_group_index for the
    //         2-pass group-atomic LOD descent).
    assert_eq!(MeshletDescriptor::SIZE, 112);
}

#[test]
fn descriptor_field_offsets_match_shader_layout() {
    use std::mem::offset_of;
    // Drift here would break meshlet_cull.wgsl / meshlet_main.wgsl
    // bind reads silently. The asserts mirror the WGSL `struct
    // MeshletDescriptor` declaration.
    assert_eq!(offset_of!(MeshletDescriptor, vertex_offset), 0);
    assert_eq!(offset_of!(MeshletDescriptor, triangle_offset), 4);
    assert_eq!(offset_of!(MeshletDescriptor, vertex_count), 8);
    assert_eq!(offset_of!(MeshletDescriptor, triangle_count), 12);
    assert_eq!(offset_of!(MeshletDescriptor, aabb_min), 16);
    assert_eq!(offset_of!(MeshletDescriptor, parent_meshlet_index), 28);
    assert_eq!(offset_of!(MeshletDescriptor, aabb_max), 32);
    assert_eq!(offset_of!(MeshletDescriptor, lod_error), 44);
    assert_eq!(offset_of!(MeshletDescriptor, bounds_center), 48);
    assert_eq!(offset_of!(MeshletDescriptor, bounding_radius), 60);
    assert_eq!(offset_of!(MeshletDescriptor, cone_apex), 64);
    assert_eq!(offset_of!(MeshletDescriptor, cone_cutoff), 76);
    assert_eq!(offset_of!(MeshletDescriptor, cone_axis), 80);
    assert_eq!(offset_of!(MeshletDescriptor, group_index), 92);
    assert_eq!(offset_of!(MeshletDescriptor, children_group_index), 96);
    assert_eq!(offset_of!(MeshletDescriptor, lod_level), 100);
}

#[test]
fn root_meshlet_sentinel_distinct_from_real_index() {
    // Any sane meshlet count fits in u32; MESHLET_ROOT_PARENT is
    // u32::MAX which can never collide with a real index because
    // wgpu storage-buffer addressing tops out below 2^32 entries.
    assert_eq!(MESHLET_ROOT_PARENT, u32::MAX);
}

#[test]
fn zeroed_descriptor_has_root_parent_via_construct() {
    // Note: bytemuck::Zeroable initialises parent_meshlet_index to 0
    // (a valid index), so callers MUST set the sentinel explicitly
    // when constructing a root meshlet. This test documents that
    // contract.
    let d = MeshletDescriptor::zeroed();
    assert_eq!(d.parent_meshlet_index, 0);
    assert_eq!(d.lod_error, 0.0);
}

#[test]
fn descriptor_constructs_via_zeroable() {
    let d = MeshletDescriptor::zeroed();
    assert_eq!(d.vertex_count, 0);
    assert_eq!(d.triangle_count, 0);
}

#[test]
fn empty_mesh_reports_zero_counts() {
    let mesh = MeshletMesh {
        vertices: Vec::new(),
        meshlet_vertices: Vec::new(),
        meshlet_triangles: Vec::new(),
        meshlets: Vec::new(),
        aabb: Aabb::empty(),
    };
    assert_eq!(mesh.meshlet_count(), 0);
    assert_eq!(mesh.total_vertex_count(), 0);
    assert_eq!(mesh.total_triangle_count(), 0);
}
