use super::*;

#[test]
fn material_params_layout_is_pod_48_bytes() {
    // 16 B base_color + 16 B (metallic, rough, emissive, pad)
    // + 16 B (albedo, normal, metal_rough, pad) = 48 B.
    assert_eq!(std::mem::size_of::<MaterialParams>(), 48);
    assert_eq!(std::mem::align_of::<MaterialParams>(), 4);
}

#[test]
fn default_material_is_white_diffuse_mid_roughness() {
    let m = MaterialParams::default();
    assert_eq!(m.base_color, [1.0, 1.0, 1.0, 1.0]);
    assert_eq!(m.metallic(), 0.0);
    assert_eq!(m.roughness(), 0.5);
    assert_eq!(m.emissive(), 0.0);
    // No maps by default — every channel carries the sentinel.
    assert_eq!(m.albedo_index(), NO_TEXTURE);
    assert_eq!(m.normal_index(), NO_TEXTURE);
    assert_eq!(m.metal_roughness_index(), NO_TEXTURE);
}

#[test]
fn texture_indices_round_trip_with_sentinel() {
    let m = MaterialParams::default().with_texture_indices(3, NO_TEXTURE, 7);
    assert_eq!(m.albedo_index(), 3);
    assert_eq!(m.normal_index(), NO_TEXTURE);
    assert_eq!(m.metal_roughness_index(), 7);
    assert_eq!(m.texture_indices[3], 0, "pad slot stays zero");
}

#[test]
fn new_packs_scalars_correctly() {
    let m = MaterialParams::new([0.2, 0.4, 0.8, 1.0], 0.7, 0.3, 1.5);
    assert_eq!(m.base_color(), [0.2, 0.4, 0.8, 1.0]);
    assert_eq!(m.metallic(), 0.7);
    assert_eq!(m.roughness(), 0.3);
    assert_eq!(m.emissive(), 1.5);
}
