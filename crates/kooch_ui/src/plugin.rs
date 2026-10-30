//! Registering [`UiDocument`] and keeping [`UiTrees`] to the documents that exist.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use crate::{UiDocument, UiTrees, prune_trees};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<UiDocument>();
            }
            resources.insert(UiTrees::default());
        });
        // Ungated: a document removed while stopped frees its tree too.
        app.add_system(Stage::PostUpdate, prune_trees);
    }

    fn name(&self) -> &str {
        "UiPlugin"
    }
}
