//! Registering [`UiDocument`] and keeping [`UiTrees`] to the documents that exist.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use kooch_core::schedule::Order;

use crate::{
    GlyphAtlas, UiActions, UiDocument, UiEvent, UiFocus, UiFonts, UiPointer, UiShaders, UiTrees,
    bind_params, bind_texts, bind_values, build_documents, draw_documents, layout_documents,
    load_fonts, load_images, load_shaders, navigate_focus, pick_pointer, prune_trees,
    restyle_documents,
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
            resources.insert(UiShaders::default());
        });
        // At build, not Startup: a project's Startup system registering actions may run first.
        app.insert_resource(UiActions::default())
            .insert_resource(UiPointer::default())
            .insert_resource(UiFocus::default())
            .add_event::<UiEvent>();
        // Before anything reads actions: a click the UI takes must read as idle to the game. On
        // last frame's rects, which is what was on screen when the click was made.
        app.add_system(Stage::PreUpdate, pick_pointer);
        app.add_ordered(
            Stage::PreUpdate,
            Order::after("pick_pointer"),
            navigate_focus,
        );
        app.add_ordered(
            Stage::PreUpdate,
            Order::after("navigate_focus"),
            bind_values,
        );
        // Ungated: a document removed while stopped frees its tree too.
        app.add_system(Stage::PostUpdate, prune_trees);
        // Before the build: a document shapes its text with the fonts registered by then.
        app.add_system(Stage::PostUpdate, load_fonts);
        app.add_ordered(
            Stage::PostUpdate,
            Order::after("load_fonts"),
            build_documents,
        );
        app.add_ordered(
            Stage::PostUpdate,
            Order::after("build_documents"),
            restyle_documents,
        );
        // Before layout: a full atlas clears, and the glyphs rasterize again against it.
        app.add_ordered(
            Stage::PostUpdate,
            Order::after("restyle_documents"),
            load_images,
        );
        app.add_ordered(Stage::PostUpdate, Order::after("load_images"), load_shaders);
        // After gameplay's `Update`, so a bound field shows this frame's value, and after the
        // build, so a new document never shows a raw `{Type.field}`.
        app.add_ordered(Stage::PostUpdate, Order::after("load_shaders"), bind_params);
        app.add_ordered(Stage::PostUpdate, Order::after("bind_params"), bind_texts);
        // After gameplay's `Update`, which is where a game edits its trees.
        app.add_ordered(
            Stage::PostUpdate,
            Order::after("bind_texts"),
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
