use super::*;

#[test]
fn disabled_instance_acquires_no_slot_and_reports_none() {
    let timers = MeshletGpuTimers::disabled();
    assert!(!timers.is_enabled());
    assert_eq!(timers.last_frame_ms(), None);
    assert_eq!(timers.last_stage_ms(0), None);
    assert!(timers.last_frame_stage_timings().is_none());
    // Even after calling drain_ready, last_frame_ms stays None
    // because no slot ever transitions to Ready on a disabled
    // instance.
    let mut timers = timers;
    timers.drain_ready();
    assert_eq!(timers.acquire_slot(), None);
    assert_eq!(timers.last_frame_ms(), None);
}

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
fn disabled_instance_reports_stage_count_one() {
    // Disabled timers still answer `stage_count()` so callers
    // using the multi-stage API can branch on `is_enabled()`
    // without first probing the count.
    let timers = MeshletGpuTimers::disabled();
    assert_eq!(timers.stage_count(), 1);
}
