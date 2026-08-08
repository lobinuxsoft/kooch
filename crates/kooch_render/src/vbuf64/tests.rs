use super::*;

#[test]
fn from_supported_round_trips() {
    assert!(Vbuf64Support::from_supported(true).is_supported());
    assert!(!Vbuf64Support::from_supported(false).is_supported());
}

#[test]
fn required_bundle_is_four_flags() {
    let bundle = required_features();
    assert!(bundle.contains(Features::TEXTURE_ATOMIC));
    assert!(bundle.contains(Features::TEXTURE_INT64_ATOMIC));
    assert!(bundle.contains(Features::SHADER_INT64));
    assert!(bundle.contains(Features::SHADER_INT64_ATOMIC_MIN_MAX));
}

#[test]
fn pack_unpack_round_trips_mid_range() {
    let cases = [
        (0.5_f32, 12_345, 7),
        (0.123_f32, 0, 0),
        (0.999_f32, MAX_CLUSTER_ID, TRI_ID_MASK),
    ];
    for (d, c, t) in cases {
        let packed = pack_visibility(d, c, t);
        let (du, cu, tu) = unpack_visibility(packed);
        assert_eq!(du.to_bits(), d.to_bits(), "depth round-trip {d}");
        assert_eq!(cu, c, "cluster_id round-trip");
        assert_eq!(tu, t, "tri_id round-trip");
    }
}

#[test]
fn pack_unpack_round_trips_reversed_z_extremes() {
    for depth in [0.0_f32, 1.0_f32, f32::MIN_POSITIVE, 1.0 - f32::EPSILON] {
        let packed = pack_visibility(depth, 17, 3);
        let (du, cu, tu) = unpack_visibility(packed);
        assert_eq!(du.to_bits(), depth.to_bits(), "depth {depth}");
        assert_eq!(cu, 17);
        assert_eq!(tu, 3);
    }
}

#[test]
fn closer_reversed_z_depth_yields_larger_packed() {
    // Reversed-Z: closer fragment has the *higher* depth value, so the
    // packed u64 must be greater for the closer fragment. This is the
    // load-bearing invariant for `textureAtomicMax` to act as
    // winner-takes-all.
    let near = pack_visibility(0.95, 10, 0);
    let far = pack_visibility(0.20, 10, 0);
    assert!(near > far, "expected near > far ({near} vs {far})");
}

#[test]
fn equal_depth_higher_cluster_id_wins_atomicmax() {
    // Bevy's tie-break: at equal depth, the fragment with the larger
    // packed_ids value wins under atomicMax. Document the behaviour so
    // the integration test for coplanar meshlets asserts the right
    // direction (larger cluster_id, not smaller).
    let lhs = pack_visibility(0.5, 100, 0);
    let rhs = pack_visibility(0.5, 99, 0);
    assert!(lhs > rhs, "tie-break: larger cluster_id wins");
}

#[test]
fn default_max_triangles_fits_tri_id_slot() {
    use crate::meshlet::DEFAULT_MAX_TRIANGLES;
    assert!(
        DEFAULT_MAX_TRIANGLES as u32 <= TRI_ID_MASK + 1,
        "DEFAULT_MAX_TRIANGLES ({DEFAULT_MAX_TRIANGLES}) overflows {TRI_ID_BITS}-bit tri_id slot"
    );
}
