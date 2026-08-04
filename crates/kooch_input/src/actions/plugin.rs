//! Wiring actions into a frame.
//!
//! # Where it sits
//!
//! Loading runs in `Stage::PreUpdate` and reading in `Stage::Input`,
//! right after the backend has been pumped and before anything in
//! `Update`. A gameplay system therefore reads the action as of *this*
//! frame, not last one.
//!
//! # No active map
//!
//! There was one, and it was a global: a component named which
//! `.inputmap` the whole session played under, gameplay looked an action
//! up **by name** inside it, and every consumer spelled that name out —
//! so renaming an action in the panel silently stopped the control.
//!
//! An action is an asset now. A component points at one by guid, picked
//! in the Inspector like a mesh, and each is enabled on its own — which a
//! map could not do, being all or nothing. Nothing here is global, and
//! nothing names an action.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;

/// Declares the input components without running any input.
///
/// The editor needs them to exist as data — to inspect them, to offer the
/// asset picker, to mirror them — while gameplay lives in the project's
/// process. Same split `CameraComponentsPlugin` makes, and for the same
/// reason: a host that authors is not a host that plays.
pub struct InputComponentsPlugin;

impl Plugin for InputComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<kooch_ecs::component::ComponentRegistry>() {
                registry.register_cpu_reflected::<super::single::InputAction>();
            }
        });
    }

    fn name(&self) -> &str {
        "InputComponentsPlugin"
    }
}

/// Loads the project's actions and reads them once per frame.
///
/// Add it after [`InputPlugin`](crate::InputPlugin): this reads what that
/// one pumps, and plugin order is system order within a stage.
#[derive(Default)]
pub struct ActionsPlugin;

impl Plugin for ActionsPlugin {
    fn build(&self, app: &mut App) {
        // The components have to exist wherever the actions do, or a
        // scene pointing at one would load into a world that cannot hold
        // the pointer.
        InputComponentsPlugin.build(app);
        app.insert_resource(super::single::LoadedActions::default());
        // Loaded before they are read, and reloaded when a file changes:
        // editing a binding in the panel takes effect without a restart.
        app.add_system(Stage::PreUpdate, super::single::load_input_actions);
        app.add_system(Stage::Input, super::single::read_input_actions);
    }

    fn name(&self) -> &str {
        "ActionsPlugin"
    }
}
