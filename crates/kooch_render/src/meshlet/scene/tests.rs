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

#[test]
fn encode_decode_round_trip() {
    for instance_id in [0u32, 1, 42, 0xFFFF] {
        for meshlet_id in [0u32, 1, 100, 0xFFFF] {
            let packed = encode_scene_visible_id(instance_id, meshlet_id);
            assert_eq!(decode_scene_visible_id(packed), (instance_id, meshlet_id));
        }
    }
}

#[test]
fn decode_extracts_high_low_halves() {
    let packed = (5u32 << 16) | 12u32;
    assert_eq!(decode_scene_visible_id(packed), (5, 12));
}
