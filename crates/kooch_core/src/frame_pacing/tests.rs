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

/// 🔴 The deadline advances by whole budgets, not from the moment it is asked. Pacing from `now`
/// would make the gap `frame_time + budget`, which is uneven by exactly the thing being removed.
#[test]
fn a_cap_paces_evenly() {
    let budget = Duration::from_millis(10);
    let mut cap = FrameCap::new(100.0);
    let start = Instant::now();

    let first = cap.deadline(start).unwrap();
    // The next frame asks late, having spent 4 ms of work after waking on `first`.
    let second = cap.deadline(first + Duration::from_millis(4)).unwrap();

    assert_eq!(first, start + budget);
    assert_eq!(second - first, budget);
}

/// An overrun owes time it can never repay, and catching up would spend the next frames in a burst.
#[test]
fn an_overrun_restarts_the_cap() {
    let mut cap = FrameCap::new(100.0);
    let start = Instant::now();
    let first = cap.deadline(start).unwrap();

    let late = first + Duration::from_millis(50);
    assert_eq!(
        cap.deadline(late).unwrap(),
        late + Duration::from_millis(10)
    );
}

/// 🔴 `from_secs_f64` panics on a non-finite argument, and `hz <= 0.0` is false for NaN.
#[test]
fn an_unusable_rate_uncaps() {
    for hz in [0.0, -60.0, f64::NAN, f64::INFINITY] {
        assert_eq!(FrameCap::new(hz).hz(), None, "{hz}");
    }
}

#[test]
fn a_cap_reads_off_as_none() {
    assert_eq!(cap_from(Some("off")), Some(0.0));
    assert_eq!(cap_from(Some(" 72 ")), Some(72.0));
    assert_eq!(cap_from(Some("nonsense")), None);
    assert_eq!(cap_from(None), None);
}
