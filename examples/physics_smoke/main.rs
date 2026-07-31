//! Drives the physics subsystem through the real engine and reports what
//! the solver ended up with.
//!
//! The unit tests assert one thing each against a hand-built `Resources`.
//! This runs the actual `App` — plugins, schedule, fixed timestep — and
//! prints the numbers, which is the difference between "the function
//! returns what I expected" and "the engine does what I expected".
//!
//! Headless on purpose: no window, no GPU. Physics does not need either,
//! and a smoke test that needs a display is one that cannot run anywhere.
//!
//! Run with:
//!
//! ```text
//! cargo run --example physics_smoke --no-default-features \
//!     --features physics,physics-debug-render
//! ```

use kooch_core::prelude::*;
use kooch_core::run_state::Playing;
use kooch_ecs::entity::Entity;
use kooch_ecs::plugin::EcsPlugin;
use kooch_physics::plugin::PhysicsPlugin;

/// How many *fixed steps* to run before reporting and quitting.
///
/// Steps, not frames. The default runner spins as fast as it can and
/// accumulates fixed steps from real elapsed time, so a frame count
/// measures how fast the machine is, not how long the scene simulated —
/// the first version of this example ran 240 frames in ten milliseconds
/// and reported, correctly, that nothing had moved.
///
/// At 60 Hz this is four seconds of simulated time, and it costs four
/// seconds of wall clock to get.
const STEPS: u64 = 240;

/// The entities the report reads back.
#[derive(Default)]
struct Cast {
    falling: Option<Entity>,
    big_sphere: Option<Entity>,
    compound: Option<Entity>,
    door: Option<Entity>,
    fuse: Option<Entity>,
    fuse_joint: Option<Entity>,
    /// Two identical cubes shoved at the same speed over floors that
    /// differ only in friction (#623).
    slippery: Option<Entity>,
    grippy: Option<Entity>,
    spinner: Option<Entity>,
    /// A trigger volume and the body falling through it (#561).
    trigger: Option<Entity>,
    /// Same collision groups as the wall, disjoint solver groups: detects
    /// it and is not stopped by it (#561).
    ghost: Option<Entity>,
}

/// What the solver reported over the whole run.
///
/// Accumulated as it arrives, because the event buffers are
/// double-buffered: reading only at the end would see the last frame's
/// events and nothing else.
#[derive(Default)]
struct Heard {
    started: usize,
    stopped: usize,
    sensor_started: usize,
    forces: usize,
    peak_force: f32,
    joint_breaks: usize,
    ghost_detections: usize,
}

fn main() {
    kooch_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(EcsPlugin);
    app.add_plugin(PhysicsPlugin::new());
    app.insert_resource(Cast::default());
    app.add_system(Stage::Startup, build_scene);
    app.add_system(Stage::Update, launch);
    app.add_system(Stage::Update, listen);
    app.add_system(Stage::Last, report);
    Playing::set(app.resources_mut(), true);
    app.run();
}

mod report;
mod scene;

use report::{launch, listen, report};
use scene::build_scene;
