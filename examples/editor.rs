//! Example: embedded editor overlay with egui.
//!
//! Opens a window with the entity inspector overlay.
//! Spawn/despawn entities and inspect their components in real time.
//!
//! Run with: cargo run --example editor --features editor

use ome_core::prelude::*;
use ome_ecs::commands::Commands;
use ome_ecs::component::Component;
use ome_ecs::EcsPlugin;
use ome_editor_core::EditorPlugin;
use ome_window::WindowPlugin;

struct Health(pub u32);
impl Component for Health {}

struct Name(pub String);
impl Component for Name {}

struct Marker;
impl Component for Marker {}

fn spawn_demo_entities(resources: &mut Resources) {
    let mut commands = resources.remove::<Commands>().expect("Commands not found");

    // Player entity with Health + Name.
    commands
        .spawn(resources)
        .insert(Health(100))
        .insert(Name("Player".into()));

    // Enemy entities with Health.
    commands
        .spawn(resources)
        .insert(Health(50))
        .insert(Name("Goblin".into()))
        .insert(Marker);

    commands
        .spawn(resources)
        .insert(Health(200))
        .insert(Name("Dragon".into()));

    // Empty entity (no components).
    commands.spawn(resources);

    commands.apply(resources);
    resources.insert(commands);

    tracing::info!("Demo entities spawned");
}

fn main() {
    ome_core::init_tracing();

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugin(WindowPlugin {
        title: "OME Editor Overlay".into(),
        width: 1280,
        height: 720,
    });
    app.add_plugin(EcsPlugin);
    app.add_plugin(EditorPlugin);
    app.add_system(Stage::Startup, spawn_demo_entities);
    app.run();
}
