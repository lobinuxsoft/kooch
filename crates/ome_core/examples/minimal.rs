//! Minimal example demonstrating ome_core functionality.
//!
//! This example runs for 3 seconds, logging frame and fixed update counts
//! every 60 frames (roughly every second at 60 FPS).
//!
//! Run with: `cargo run --example minimal`

use std::time::Duration;

use ome_core::prelude::*;

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(FrameCounter::default());
    app.add_system(Stage::Startup, startup);
    app.add_system(Stage::Update, update);
    app.set_runner(test_runner);
    app.run();
}

fn startup(_resources: &mut Resources) {
    tracing::info!("Startup system executed!");
}

fn update(resources: &mut Resources) {
    let (frames, fixed) = {
        let time = resources.get::<Time>().unwrap();
        (time.frame_count(), time.fixed_count())
    };

    let counter = resources.get_mut::<FrameCounter>().unwrap();
    counter.frames = frames;
    counter.fixed = fixed;

    // Log every 60 frames
    if frames > 0 && frames % 60 == 0 {
        tracing::info!("Frames: {}, Fixed: {}", frames, fixed);
    }
}

/// Simple frame counter resource for tracking execution.
#[derive(Default)]
struct FrameCounter {
    frames: u64,
    fixed: u64,
}

/// Custom test runner that simulates 180 frames at 60 FPS.
///
/// Instead of using real time, we advance by exactly 1/60 second each frame.
/// This makes the test deterministic.
fn test_runner(mut app: App) {
    // Run startup
    app.schedule.run_startup(&mut app.resources);

    let frame_delta = Duration::from_secs_f64(1.0 / 60.0);

    for _ in 0..180 {
        // Update events
        if let Some(events) = app.resources.get_mut::<Events<AppExit>>() {
            events.update();
        }

        // Check for exit
        if app
            .resources
            .get::<Events<AppExit>>()
            .map(|e| !e.is_empty())
            .unwrap_or(false)
        {
            tracing::info!("AppExit received, shutting down");
            break;
        }

        // Advance time by fixed amount
        let fixed_steps = {
            let time = app.resources.get_mut::<Time>().unwrap();
            time.advance(frame_delta)
        };

        // Run pre-physics stages
        app.schedule.run_pre_physics(&mut app.resources);

        // Run fixed stages
        for _ in 0..fixed_steps {
            app.schedule.run_fixed_stages(&mut app.resources);
        }

        // Run post-physics stages
        app.schedule.run_post_physics(&mut app.resources);
    }

    // Final stats
    let counter = app.resources.get::<FrameCounter>().unwrap();
    tracing::info!(
        "Final: {} frames, {} fixed updates",
        counter.frames,
        counter.fixed
    );

    tracing::info!("AppExit received, shutting down");
}
