use super::*;
use crate::stage::Stage;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

fn test_app() -> App {
    let mut app = App::new();
    app.insert_resource(Time::new());
    app
}

#[test]
fn run_once_executes_stages() {
    let mut app = test_app();

    let startup_count = Arc::new(AtomicU32::new(0));
    let update_count = Arc::new(AtomicU32::new(0));

    let sc = startup_count.clone();
    app.add_system(Stage::Startup, move |_| {
        sc.fetch_add(1, Ordering::SeqCst);
    });

    let uc = update_count.clone();
    app.add_system(Stage::Update, move |_| {
        uc.fetch_add(1, Ordering::SeqCst);
    });

    run_once(app);

    assert_eq!(startup_count.load(Ordering::SeqCst), 1);
    assert_eq!(update_count.load(Ordering::SeqCst), 1);
}

#[test]
fn run_for_frames_correct_count() {
    let mut app = test_app();

    let frame_count = Arc::new(AtomicU32::new(0));

    let fc = frame_count.clone();
    app.add_system(Stage::Update, move |_| {
        fc.fetch_add(1, Ordering::SeqCst);
    });

    run_for_frames(app, 5);

    assert_eq!(frame_count.load(Ordering::SeqCst), 5);
}

#[test]
fn app_exit_stops_loop() {
    let mut app = test_app();

    let frame_count = Arc::new(AtomicU32::new(0));

    let fc = frame_count.clone();
    app.add_system(Stage::Update, move |resources| {
        let count = fc.fetch_add(1, Ordering::SeqCst);
        // Request exit after 3 frames
        if count >= 2 {
            if let Some(events) = resources.get_mut::<Events<AppExit>>() {
                events.send(AppExit);
            }
        }
    });

    run_for_frames(app, 100);

    // Should stop after AppExit is sent
    // Frame 0, 1, 2 run, then exit on frame 3
    assert!(frame_count.load(Ordering::SeqCst) <= 4);
}

/// A project hosting the editor is headless — no window, no vsync —
/// and used to spin a core flat out between edits. It sleeps now,
/// and the thing that wakes it is a request arriving on its socket.
#[test]
fn a_waiting_loop_sleeps_until_woken() {
    use crate::frame_pacing::{FramePace, FrameRequest, FrameWaker};

    let mut app = test_app();
    let waker = app.resources.get::<FrameWaker>().cloned().expect("waker");
    app.insert_resource(FrameRequest::new(FramePace::Wait));

    let frames = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&frames);
    app.add_system(Stage::Update, move |resources| {
        // Three frames, each one only reachable by being woken.
        if counter.fetch_add(1, Ordering::SeqCst) >= 2
            && let Some(events) = resources.get_mut::<Events<AppExit>>()
        {
            events.send(AppExit);
        }
    });

    // Nothing else can end this: the loop blocks on the waker, and
    // only these wakes let it reach the frame that asks to exit.
    let ticker = waker.clone();
    let seen = Arc::clone(&frames);
    std::thread::spawn(move || {
        while seen.load(Ordering::SeqCst) < 4 {
            ticker.wake();
            std::thread::sleep(Duration::from_millis(1));
        }
    });

    default_runner(app);
    assert!(
        frames.load(Ordering::SeqCst) >= 3,
        "the loop never woke up often enough to finish",
    );
}

/// The pre-#656 contract: an app with no opinion keeps spinning, and
/// nothing has to wake it.
#[test]
fn a_loop_without_a_frame_request_still_spins() {
    let mut app = test_app();
    let frames = Arc::new(AtomicU32::new(0));
    let counter = Arc::clone(&frames);
    app.add_system(Stage::Update, move |resources| {
        if counter.fetch_add(1, Ordering::SeqCst) >= 4
            && let Some(events) = resources.get_mut::<Events<AppExit>>()
        {
            events.send(AppExit);
        }
    });

    // No waker thread anywhere: if this returns, it never slept.
    default_runner(app);
    assert!(frames.load(Ordering::SeqCst) >= 5);
}

#[test]
fn fixed_steps_calculated_correctly() {
    let mut app = test_app();

    // Set up time to track via advance
    if let Some(time) = app.resources.get_mut::<Time>() {
        // 100ms at 60Hz should give 5-6 fixed steps (float precision dependent)
        let steps = time.advance(Duration::from_millis(100));
        assert!(
            steps >= 5 && steps <= 6,
            "Expected 5-6 steps, got {}",
            steps
        );
    }
}
