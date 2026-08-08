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
mod tests;
