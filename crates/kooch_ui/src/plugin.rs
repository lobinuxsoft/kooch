//! Registering [`UiDocument`] and keeping [`UiTrees`] to the documents that exist.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use kooch_core::schedule::Order;

use crate::{
    GlyphAtlas, UiDocument, UiFonts, UiTrees, build_documents, draw_documents, layout_documents,
    prune_trees,
};

/// Only the components: what the editor needs to offer and inspect them, without running any UI.
pub struct UiComponentsPlugin;

impl Plugin for UiComponentsPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<UiDocument>();
            }
        });
    }

    fn name(&self) -> &str {
        "UiComponentsPlugin"
    }
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugin(UiComponentsPlugin);
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            resources.insert(UiTrees::default());
            resources.insert(UiFonts::default());
            resources.insert(GlyphAtlas::default());
        });
        // Ungated: a document removed while stopped frees its tree too.
        app.add_system(Stage::PostUpdate, prune_trees);
        app.add_system(Stage::PostUpdate, build_documents);
        // After gameplay's `Update`, which is where a game edits its trees.
        app.add_ordered(
            Stage::PostUpdate,
            Order::after("build_documents"),
            layout_documents,
        );
        app.add_ordered(
            Stage::PostUpdate,
            Order::after("layout_documents"),
            draw_documents,
        );
    }

    fn name(&self) -> &str {
        "UiPlugin"
    }
}
