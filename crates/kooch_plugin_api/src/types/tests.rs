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

/// 🔴 The host's parity test iterates this list to prove every stage maps, so a variant missing
/// from it makes an unmapped stage invisible. Order is not what it guards — `PrePhysics` is
/// declared last and runs fifth.
#[test]
fn all_holds_every_stage() {
    assert_eq!(Stage::ALL.len(), 15);
    let mut seen = Stage::ALL.to_vec();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), Stage::ALL.len(), "a stage is listed twice");
    for stage in [Stage::Startup, Stage::Last, Stage::PrePhysics] {
        assert!(Stage::ALL.contains(&stage), "{stage:?} is missing");
    }
}
