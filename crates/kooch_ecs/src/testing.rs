//! Components and systems that exist to MEASURE the engine, not to ship
//! in a game.
//!
//! # Why this is a feature and not just a module
//!
//! A shipped game may not carry what it does not use — the same rule
//! `physics-debug-render` follows, and the reason #558 exists. With
//! `testing` off, nothing below is compiled: the component is not
//! registered, the system is not scheduled, and the type name never
//! reaches a scene file.
//!
//! # What belongs here
//!
//! Only things a *measurement* needs and a game does not. The bar is
//! whether removing it from a release build could change what a player
//! sees; if it could, it is not a testing helper.
//!
//! 🔴 A component here still writes its type name into any scene that
//! uses it. Turning the feature off later does not corrupt those scenes
//! — an unregistered component is dropped on load rather than erroring
//! — but the entity comes back without it, so a benchmark scene saved
//! with `testing` on and opened with it off is a scene whose lights
//! have stopped moving, silently. That is the intended failure and it
//! is why nothing a game needs may live here.

pub mod spin;

use crate::component::ComponentRegistry;
use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::stage::Stage;

/// Registers everything in this module.
///
/// Added by `DefaultPlugins` only when the `testing` feature is on, so a
/// build without it has no way to reach any of this.
pub struct TestingPlugin;

impl Plugin for TestingPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, register_testing_components);
        // 🔴 `Update`, and not the default talking. This writes a local
        // `Transform`; whatever orbits reads its `GlobalTransform`,
        // which the engine resolves during `PostUpdate`. Written after
        // that, every orbiting light renders one frame behind its pivot
        // forever — shadows that lag the camera, with nothing on screen
        // to say why.
        app.add_system(Stage::Update, spin::spin_pivots);
    }
}

fn register_testing_components(resources: &mut kooch_core::resource::Resources) {
    if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
        registry.register_cpu_reflected::<spin::Spin>();
    }
}
