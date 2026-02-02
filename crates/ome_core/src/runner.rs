//! Game loop runners.
//!
//! A runner takes ownership of the `App` and controls the main loop.
//! The default runner implements "Fix Your Timestep" with fixed physics
//! and variable rendering.

use crate::app::App;
use crate::event::{AppExit, Events};
use crate::stage::Stage;
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
///    d. Run frame stages (First → PostUpdate)
///    e. Run GPU stages (GpuSync, Gpu)
///    f. Run fixed stages N times (Physics, PostPhysics)
///    g. Run render stages (PreRender → Last)
/// ```
pub fn default_runner(mut app: App) {
    // Run startup systems once
    app.schedule.run_startup(&mut app.resources);

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

    // Run all frame stages once
    app.schedule.run_frame_stages(&mut app.resources);

    // Run fixed stages once
    app.schedule.run_fixed_stages(&mut app.resources);
}

/// Updates all event buffers.
fn update_events(app: &mut App) {
    // Update AppExit events
    if let Some(events) = app.resources.get_mut::<Events<AppExit>>() {
        events.update();
    }
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

        app.schedule.run_frame_stages(&mut app.resources);
        app.schedule.run_fixed_stages(&mut app.resources);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
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

    #[test]
    fn fixed_steps_calculated_correctly() {
        let mut app = test_app();

        // Set up time to track via advance
        if let Some(time) = app.resources.get_mut::<Time>() {
            // 100ms at 60Hz should give ~6 fixed steps
            let steps = time.advance(Duration::from_millis(100));
            assert_eq!(steps, 6);
        }
    }
}
