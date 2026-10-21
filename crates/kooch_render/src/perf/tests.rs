use super::*;

/// The whole contract: the last measurement wins, in both directions.
#[test]
fn the_last_total_wins() {
    let tracker = EngineVramTracker::new();
    tracker.publish(3072);
    assert_eq!(tracker.bytes(), 3072);
    tracker.publish(1024);
    assert_eq!(
        tracker.bytes(),
        1024,
        "a figure that cannot fall is a leak report"
    );
}
