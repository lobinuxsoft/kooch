use super::*;

#[test]
fn mip_count_for_square_pow2() {
    assert_eq!(mip_count_for(1, 1), 1);
    assert_eq!(mip_count_for(2, 2), 2);
    assert_eq!(mip_count_for(4, 4), 3);
    assert_eq!(mip_count_for(64, 64), 7);
    assert_eq!(mip_count_for(1024, 1024), 11);
}

#[test]
fn mip_count_for_non_pow2_rounds_up() {
    assert_eq!(mip_count_for(3, 3), 3);
    assert_eq!(mip_count_for(5, 7), 4);
    assert_eq!(mip_count_for(1920, 1080), 12);
}

#[test]
fn mip_count_caps_at_max() {
    assert_eq!(mip_count_for(u32::MAX, u32::MAX), MAX_MIP_COUNT);
}

#[test]
fn mip_size_halves_with_floor_min_one() {
    assert_eq!(mip_size(64, 32, 0), (64, 32));
    assert_eq!(mip_size(64, 32, 1), (32, 16));
    assert_eq!(mip_size(64, 32, 5), (2, 1));
    assert_eq!(mip_size(64, 32, 6), (1, 1));
    assert_eq!(mip_size(64, 32, 100), (1, 1));
}
