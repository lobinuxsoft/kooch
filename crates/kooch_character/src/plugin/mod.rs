//! Holding the capsule up, and publishing what it is standing on.

pub mod cling;
mod hold;
pub mod leap;
pub mod sense;
pub mod turn;
pub mod walk;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::run_state::run_if_playing;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use crate::controller::CharacterController;
use crate::facing::Facing;
use crate::grounded::Grounded;
use crate::jump::{Jump, WallJump};
use crate::sprint::Sprint;
use crate::touching::Touching;
use crate::walk::Walk;
use crate::wall_slide::WallSlide;

pub use cling::cling_and_leap;
pub use hold::hold_characters;

/// The components without the system, for a host that authors characters
/// but does not simulate them.
///
/// The editor is that host, for the same reason it is
/// `GravityComponentsPlugin`'s: gameplay runs in the project's process,
/// so this side needs the fields to exist as data and must never push a
/// body with them.
pub struct CharacterComponentsPlugin;

impl Plugin for CharacterComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<CharacterController>();
                registry.register_cpu_reflected::<Facing>();
                registry.register_cpu_reflected::<Grounded>();
                registry.register_cpu_reflected::<Jump>();
                registry.register_cpu_reflected::<Sprint>();
                registry.register_cpu_reflected::<WallJump>();
                registry.register_cpu_reflected::<WallSlide>();
                registry.register_cpu_reflected::<Touching>();
                registry.register_cpu_reflected::<Walk>();
            }
        });
    }

    fn name(&self) -> &str {
        "CharacterComponentsPlugin"
    }
}

/// Registers the character components and the system that holds them up.
pub struct CharacterPlugin;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(CharacterComponentsPlugin);
        // In the fixed stage beside the solver and before it steps, for
        // the same reason gravity is: the impulse is for *this* step, and
        // applying it afterwards moves the body a step late.
        //
        // After `GravityPlugin`, which is registered first by the facade:
        // the spring fights gravity, and fighting last step's gravity is
        // a character that sinks whenever the field changes.
        app.add_system(Stage::Physics, run_if_playing(hold_characters));
        // After it, and for the whole reason the sense pass exists: a
        // wall slide and a jump read what `hold_characters` found this
        // step rather than probing for it again.
        app.add_system(Stage::Physics, run_if_playing(cling::cling_and_leap));
    }

    fn name(&self) -> &str {
        "CharacterPlugin"
    }
}
