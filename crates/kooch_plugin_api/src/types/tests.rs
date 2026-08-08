use super::*;

#[test]
fn pack_unpack_roundtrip() {
    let (idx, generation) = (42, 7);
    assert_eq!(
        unpack_entity(pack_entity(idx, generation)),
        (idx, generation)
    );
}

#[test]
fn pack_unpack_zero() {
    assert_eq!(unpack_entity(pack_entity(0, 0)), (0, 0));
}

#[test]
fn pack_unpack_max() {
    let handle = pack_entity(u32::MAX, u32::MAX);
    assert_eq!(unpack_entity(handle), (u32::MAX, u32::MAX));
}

#[test]
fn pack_layout() {
    let handle = pack_entity(0xDEAD, 0xBEEF);
    assert_eq!(handle & 0xFFFF_FFFF, 0xDEAD);
    assert_eq!(handle >> 32, 0xBEEF);
}

#[test]
fn all_lists_every_stage_in_order() {
    assert_eq!(Stage::ALL.len(), 14);
    assert_eq!(Stage::ALL[0], Stage::Startup);
    assert_eq!(Stage::ALL[13], Stage::Last);
    assert!(Stage::Update < Stage::Render);
}
