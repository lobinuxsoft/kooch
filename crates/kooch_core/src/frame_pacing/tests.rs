use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

#[test]
fn continuous_beats_everything() {
    assert_eq!(
        FramePace::Wait.most_urgent(FramePace::Continuous),
        FramePace::Continuous
    );
    assert_eq!(
        FramePace::After(Duration::from_millis(1)).most_urgent(FramePace::Continuous),
        FramePace::Continuous
    );
}

#[test]
fn shorter_deadline_wins_and_wait_never_lowers() {
    let short = Duration::from_millis(5);
    let long = Duration::from_millis(500);
    assert_eq!(
        FramePace::After(long).most_urgent(FramePace::After(short)),
        FramePace::After(short)
    );
    assert_eq!(
        FramePace::After(long).most_urgent(FramePace::Wait),
        FramePace::After(long)
    );
}

#[test]
fn repaint_delay_maps_to_the_three_cases() {
    assert_eq!(
        FramePace::from_repaint_delay(Duration::ZERO),
        FramePace::Continuous
    );
    assert_eq!(
        FramePace::from_repaint_delay(Duration::MAX),
        FramePace::Wait
    );
    assert_eq!(
        FramePace::from_repaint_delay(Duration::from_millis(16)),
        FramePace::After(Duration::from_millis(16))
    );
}

#[test]
fn take_resets_to_baseline() {
    let mut request = FrameRequest::new(FramePace::Wait);
    request.request(FramePace::Continuous);
    assert_eq!(request.take(), FramePace::Continuous);
    assert_eq!(request.take(), FramePace::Wait);
}

#[test]
fn a_continuous_frame_survives_a_later_wait() {
    // One system animating outvotes every system that has nothing
    // to say — otherwise draw order would decide whether the UI
    // animates, which is not a thing anyone can debug.
    let mut request = FrameRequest::new(FramePace::Wait);
    request.request(FramePace::Continuous);
    request.request(FramePace::Wait);
    assert_eq!(request.take(), FramePace::Continuous);
}

#[test]
fn a_spinning_baseline_never_falls_asleep() {
    let mut request = FrameRequest::new(FramePace::Continuous);
    assert_eq!(request.take(), FramePace::Continuous);
    request.request(FramePace::Wait);
    assert_eq!(request.take(), FramePace::Continuous);
}

#[test]
fn wake_is_sticky_without_a_notify() {
    let waker = FrameWaker::default();
    assert!(!waker.take_pending());
    waker.wake();
    assert!(waker.take_pending());
    assert!(!waker.take_pending());
}

#[test]
fn wake_reaches_the_installed_notify_from_another_thread() {
    let waker = FrameWaker::default();
    let hits = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&hits);
    waker.set_notify(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });

    let remote = waker.clone();
    std::thread::spawn(move || remote.wake()).join().unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert!(waker.take_pending());
}
