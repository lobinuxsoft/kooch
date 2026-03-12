//! Example: embedded editor overlay with egui.
//!
//! Opens a window with the entity inspector overlay.
//! Spawn/despawn entities and inspect their components in real time.
//!
//! Run with: cargo run --example editor --features editor

use ome_core::prelude::*;
use ome_ecs::commands::Commands;
use ome_ecs::component::Component;
use ome_ecs::reflect::{FieldKind, FieldMeta, Reflect, ReflectError, ReflectValue};
use ome_ecs::EcsPlugin;
use ome_editor_core::EditorPlugin;
use ome_window::WindowPlugin;

// ---------------------------------------------------------------------------
// Demo components with Reflect
// ---------------------------------------------------------------------------

struct Health {
    pub hp: u32,
    pub max_hp: u32,
}
impl Component for Health {}

impl Reflect for Health {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[
            FieldMeta {
                name: "hp",
                type_name: "u32",
                kind: FieldKind::U32,
            },
            FieldMeta {
                name: "max_hp",
                type_name: "u32",
                kind: FieldKind::U32,
            },
        ];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "hp" => Some(ReflectValue::U32(self.hp)),
            "max_hp" => Some(ReflectValue::U32(self.max_hp)),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match field {
            "hp" => match value {
                ReflectValue::U32(v) => { self.hp = v; Ok(()) }
                other => Err(ReflectError::TypeMismatch {
                    field: "hp".into(), expected: FieldKind::U32, got: other.kind(),
                }),
            },
            "max_hp" => match value {
                ReflectValue::U32(v) => { self.max_hp = v; Ok(()) }
                other => Err(ReflectError::TypeMismatch {
                    field: "max_hp".into(), expected: FieldKind::U32, got: other.kind(),
                }),
            },
            _ => Err(ReflectError::FieldNotFound(field.into())),
        }
    }

    fn reflect_default() -> Self {
        Health { hp: 100, max_hp: 100 }
    }
}

struct Name {
    pub value: String,
}
impl Component for Name {}

impl Reflect for Name {
    fn reflect_fields(&self) -> &'static [FieldMeta] {
        static FIELDS: &[FieldMeta] = &[FieldMeta {
            name: "value",
            type_name: "String",
            kind: FieldKind::String,
        }];
        FIELDS
    }

    fn reflect_get(&self, field: &str) -> Option<ReflectValue> {
        match field {
            "value" => Some(ReflectValue::String(self.value.clone())),
            _ => None,
        }
    }

    fn reflect_set(&mut self, field: &str, value: ReflectValue) -> Result<(), ReflectError> {
        match field {
            "value" => match value {
                ReflectValue::String(v) => { self.value = v; Ok(()) }
                other => Err(ReflectError::TypeMismatch {
                    field: "value".into(), expected: FieldKind::String, got: other.kind(),
                }),
            },
            _ => Err(ReflectError::FieldNotFound(field.into())),
        }
    }

    fn reflect_default() -> Self {
        Name { value: String::new() }
    }
}

/// Marker component without reflection (inspector shows "no reflection").
struct Marker;
impl Component for Marker {}

// ---------------------------------------------------------------------------
// Demo setup
// ---------------------------------------------------------------------------

fn spawn_demo_entities(resources: &mut Resources) {
    let mut commands = resources.remove::<Commands>().expect("Commands not found");

    // Player entity with Health + Name (reflected).
    commands
        .spawn(resources)
        .insert_reflected(Health { hp: 100, max_hp: 100 })
        .insert_reflected(Name { value: "Player".into() });

    // Enemy entities with Health (reflected) + Marker (not reflected).
    commands
        .spawn(resources)
        .insert_reflected(Health { hp: 50, max_hp: 50 })
        .insert_reflected(Name { value: "Goblin".into() })
        .insert(Marker);

    commands
        .spawn(resources)
        .insert_reflected(Health { hp: 200, max_hp: 200 })
        .insert_reflected(Name { value: "Dragon".into() });

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
