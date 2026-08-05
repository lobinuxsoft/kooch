//! Game loop runners.
//!
//! A runner takes ownership of the `App` and controls the main loop.
//! The default runner implements "Fix Your Timestep" with fixed physics
//! and variable rendering.

use crate::app::App;
use crate::event::{AppExit, Events};
use crate::frame_pacing::{FramePace, FrameRequest, FrameWaker};
use crate::time::Time;

/// A function that takes ownership of the app and runs it.
///
/// Runners control the main game loop. The default runner implements
/// fixed timestep physics. Window-based apps (like winit) typically
/// override this to integrate with the platform event loop.
pub type Runner = fn(App);

/// Default runner implementing "Fix Your Timestep" game loop.
///
/// # Loop Structure
/// ```text
/// 1. Startup (once)
/// 2. Loop:
///    a. Update events (swap buffers)
///    b. Check AppExit
///    c. Update time, get fixed step count
///    d. Run frame stages (First → Update)
///    e. Run fixed stages N times (Physics, PostPhysics)
///    f. Run the rest (PostUpdate, GpuSync, Gpu, PreRender → Last)
/// ```
///
/// The fixed loop sits between `Update` and `PostUpdate` so transform
/// propagation and the GPU upload see the poses the solver produced this
/// frame, not the previous one.
///
/// # Pacing
///
/// With no [`FrameRequest`] in `Resources` this spins as fast as it can,
/// which is what a game wants and what it always did. With one, the loop
/// sleeps between frames on the terms each frame sets (#656).
///
/// That mattered more than it sounds: a project hosting the editor runs
/// here — headless, so no window and no vsync — and spun a core flat out
/// to mirror a scene nobody was editing.
pub fn default_runner(mut app: App) {
    // Run startup systems once
    app.schedule.run_startup(&mut app.resources);

    // Cloned once: the loop asks it to sleep on every frame, and it is a
    // handle to shared state rather than the state itself.
    let waker = app.resources.get::<FrameWaker>().cloned();

    loop {
        // Update all event buffers (swap read/write)
        update_events(&mut app);

        // Check for exit request
        if should_exit(&app) {
            tracing::info!("AppExit received, shutting down");
            break;
        }

        // Update time and get number of fixed steps needed
        let fixed_steps = {
            let time = app
                .resources
                .get_mut::<Time>()
                .expect("Time resource not found");
            time.update()
        };

        // Run pre-physics frame stages
        app.schedule.run_pre_physics(&mut app.resources);

        // Run fixed timestep stages (may run multiple times)
        for _ in 0..fixed_steps {
            app.schedule.run_fixed_stages(&mut app.resources);
        }

        // Run post-physics frame stages (including render)
        app.schedule.run_post_physics(&mut app.resources);

        sleep_until_the_next_frame_is_wanted(&mut app, waker.as_ref());
    }
}

/// Parks the loop for as long as this frame said the next one could wait.
///
/// A missing [`FrameRequest`] means the app never opted into idling, so
/// this returns immediately and the loop spins as before.
///
/// Without a [`FrameWaker`] there is nothing that could interrupt a
/// sleep, so a `Wait` would never end — it degrades to spinning rather
/// than hanging. The waker is inserted by `App::new`, so that is a
/// hand-built `App`, not the normal path.
fn sleep_until_the_next_frame_is_wanted(app: &mut App, waker: Option<&FrameWaker>) {
    let Some(pace) = app
        .resources
        .get_mut::<FrameRequest>()
        .map(FrameRequest::take)
    else {
        return;
    };
    let Some(waker) = waker else {
        return;
    };

    match pace {
        FramePace::Continuous => {
            // Still clear the flag: a wake that arrived during a frame we
            // were going to run anyway is already satisfied, and leaving
            // it set would waste the next chance to sleep.
            waker.take_pending();
        }
        FramePace::After(delay) => {
            waker.wait(Some(delay));
        }
        FramePace::Wait => {
            waker.wait(None);
        }
    }
}

/// Test runner that runs one frame then exits.
///
/// Useful for unit tests that need to verify system behavior.
pub fn run_once(mut app: App) {
    // Run startup
    app.schedule.run_startup(&mut app.resources);

    // Update events
    update_events(&mut app);

    // Update time
    if let Some(time) = app.resources.get_mut::<Time>() {
        time.update();
    }

    // One frame, fixed loop in its place between Update and PostUpdate.
    app.schedule.run_pre_physics(&mut app.resources);
    app.schedule.run_fixed_stages(&mut app.resources);
    app.schedule.run_post_physics(&mut app.resources);
}

/// Updates all event buffers.
fn update_events(app: &mut App) {
    // Every registered type, by asking rather than by name — see
    // `event::update_all_events` for what the hardcoded version cost.
    crate::event::update_all_events(&mut app.resources);
}

/// Returns `true` if an AppExit event has been sent.
fn should_exit(app: &App) -> bool {
    app.resources
        .get::<Events<AppExit>>()
        .map(|events| !events.is_empty())
        .unwrap_or(false)
}

/// Runs the app for a specified number of frames.
///
/// Useful for integration tests.
pub fn run_for_frames(mut app: App, frame_count: u32) {
    app.schedule.run_startup(&mut app.resources);

    for _ in 0..frame_count {
        update_events(&mut app);

        if should_exit(&app) {
            break;
        }

        if let Some(time) = app.resources.get_mut::<Time>() {
            time.update();
        }

        app.schedule.run_pre_physics(&mut app.resources);
        app.schedule.run_fixed_stages(&mut app.resources);
        app.schedule.run_post_physics(&mut app.resources);
    }
}

#[cfg(test)]
mod tests {
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
}
