use super::*;

#[test]
fn mesh_instance_layout_is_pod_96_bytes() {
    assert_eq!(std::mem::size_of::<MeshInstance>(), 96);
}

#[test]
fn scene_cull_params_layout_is_pod_16_bytes() {
    assert_eq!(std::mem::size_of::<SceneCullParams>(), 16);
}

#[test]
fn mesh_instance_round_trip_transform() {
    let m = Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0));
    let inst = MeshInstance::new(m, 7, 11);
    let recovered = inst.transform_mat4();
    // Compare every column to avoid a float-eq trap on Mat4.
    for col in 0..4 {
        for row in 0..4 {
            assert_eq!(inst.transform[col][row], recovered.col(col)[row]);
        }
    }
    assert_eq!(inst.mesh_id, 7);
    assert_eq!(inst.material_id, 11);
}

/// The culls are handed everything before the first transparent instance.
#[test]
fn transparent_instances_are_counted_out() {
    use glam::Mat4;
    let mut glass = MeshInstance::new(Mat4::IDENTITY, 0, 0);
    glass.flags |= INSTANCE_TRANSPARENT;
    let solid = MeshInstance::new(Mat4::IDENTITY, 0, 0);
    assert_eq!(opaque_count(&[solid, solid, glass]), 2);
    assert_eq!(opaque_count(&[solid]), 1);
}
