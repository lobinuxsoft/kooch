//! Example: embedded editor overlay with egui.
//!
//! Opens a window with the entity inspector overlay.
//! Spawn/despawn entities and inspect their components in real time.
//!
//! Run with: cargo run --example editor --features editor

use glam::{Quat, Vec3};
use ome_core::prelude::*;
use ome_ecs::commands::Commands;
use ome_ecs::component::Component;
use ome_ecs::transform::Transform;
use ome_ecs::{EcsPlugin, Reflect};
use ome_editor_core::EditorPlugin;
use ome_window::WindowPlugin;

// ---------------------------------------------------------------------------
// Demo components
// ---------------------------------------------------------------------------

#[derive(Default, Reflect)]
struct Health {
    pub hp: u32,
    pub max_hp: u32,
}
impl Component for Health {}

#[derive(Default, Reflect)]
struct Name {
    pub value: String,
}
impl Component for Name {}

/// Marker/tag component — zero fields.
#[derive(Default, Reflect)]
struct Marker;
impl Component for Marker {}

// ---------------------------------------------------------------------------
// Demo setup
// ---------------------------------------------------------------------------

fn spawn_demo_entities(resources: &mut Resources) {
    let mut commands = resources.remove::<Commands>().expect("Commands not found");

    // Player entity.
    commands
        .spawn(resources)
        .insert_reflected(Health { hp: 100, max_hp: 100 })
        .insert_reflected(Name { value: "Player".into() })
        .insert_reflected(Transform::from_position(Vec3::ZERO));

    // Enemy entities.
    commands
        .spawn(resources)
        .insert_reflected(Health { hp: 50, max_hp: 50 })
        .insert_reflected(Name { value: "Goblin".into() })
        .insert_reflected(Transform::from_position(Vec3::new(5.0, 0.0, 3.0)))
        .insert_reflected(Marker);

    commands
        .spawn(resources)
        .insert_reflected(Health { hp: 200, max_hp: 200 })
        .insert_reflected(Name { value: "Dragon".into() })
        .insert_reflected(Transform::new(
            Vec3::new(-10.0, 5.0, 0.0),
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
            Vec3::new(3.0, 3.0, 3.0),
        ));

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
