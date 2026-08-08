use super::*;

#[test]
fn slot_count_invariant() {
    // The state-machine reasoning in the module doc assumes ≥ 2
    // slots so a slot is always free while another is in flight.
    // 3 gives a comfortable safety margin against driver-thread
    // callback latency. Lock the constant so a future regression
    // (someone setting it to 1 to "save memory") trips this.
    assert!(SLOT_COUNT >= 2, "ring must have at least 2 slots");
}

#[test]
fn readback_size_matches_four_u32_counters() {
    // Cull shader writes 4 atomic u32 — readback must size to
    // exactly that or the bytemuck::cast_slice in drain_ready
    // panics on length mismatch.
    assert_eq!(READBACK_BYTES, 16);
    assert_eq!(
        READBACK_BYTES as usize,
        std::mem::size_of::<CullStageCounts>()
    );
}
