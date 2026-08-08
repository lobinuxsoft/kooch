use super::*;

#[test]
fn add_then_read() {
    let t = EngineVramTracker::new();
    t.add(1024);
    t.add(2048);
    assert_eq!(t.bytes(), 3072);
}

#[test]
fn sub_saturates_at_zero() {
    let t = EngineVramTracker::new();
    t.add(100);
    t.sub(200);
    assert_eq!(t.bytes(), 0, "sub past zero must clamp, not wrap");
}

#[test]
fn reset_zeroes_the_counter() {
    let t = EngineVramTracker::new();
    t.add(9999);
    t.reset();
    assert_eq!(t.bytes(), 0);
}
