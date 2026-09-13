//! Wiring actions into a frame: loaded in `Stage::PreUpdate`, read in `Stage::Input` after the
//! backend is pumped, so `Update` sees this frame.
//! No global map: actions are assets referenced by guid.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;

/// Declares the input components without running input — for the editor, which inspects and mirrors
/// them while gameplay runs in the project.
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

/// Loads the project's actions and reads them once per frame; add it after
/// [`InputPlugin`](crate::InputPlugin), whose output it reads.
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
