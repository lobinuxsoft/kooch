//! Does the Inspector hand the same widget the same id every frame?
//!
//! egui checks this itself and complains — `Widget rect … changed id
//! between passes` — but only at runtime, in debug, into a log nobody
//! reads until there are three hundred of them (#641).
//!
//! The complaint is not cosmetic. egui addresses interaction state by id:
//! what is focused, what is being dragged, which text you had selected.
//! A widget whose id changes has none of that carried over, so a drag ends
//! itself and a text cursor jumps home.
//!
//! # What this catches
//!
//! egui compares the previous pass to the current one. With a single pass
//! per frame — the ordinary case — that is **frame against frame**. So the
//! test draws the same unchanging data several times and asks egui whether
//! anything moved. Nothing about the data changes between the frames, so
//! any complaint is the Inspector's own doing.

use std::sync::Mutex;

use ome_ecs::component::ComponentId;
use ome_ecs::entity::Entity;
use ome_ecs::reflect::{EntityRef, InspectorVisibility, ReflectValue};

use super::{RotationDisplayMode, draw_inspector_content};
use crate::panels::id_stability_probe::{drawing, install_logger};
use crate::state::{ComponentDisplayInfo, EntityDisplayInfo};

/// Serialises against the other id-stability tests: the log is global.
static LOCK: Mutex<()> = Mutex::new(());

fn component(name: &str, fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
    ComponentDisplayInfo {
        type_id: std::any::TypeId::of::<()>(),
        component: ComponentId(name.len() as u32),
        short_name: name.to_owned(),
        fields: Some(fields),
        field_metas: None,
        visibility: InspectorVisibility::Editable,
    }
}

fn named(index: u32, name: &str, mut extra: Vec<ComponentDisplayInfo>) -> EntityDisplayInfo {
    let mut components = vec![component(
        "Name",
        vec![("value".into(), ReflectValue::String(name.to_owned()))],
    )];
    components.append(&mut extra);
    EntityDisplayInfo {
        entity: Entity::new(index, 0),
        components,
        parent: None,
        children: Vec::new(),
        depth: 0,
        global_rotation: None,
        scene: None,
        parent_global_rotation: None,
    }
}

/// A scene shaped like the one that produced #641: a body, and a joint
/// pointing at it.
fn scene() -> Vec<EntityDisplayInfo> {
    vec![
        named(
            0,
            "Door frame",
            vec![
                component(
                    "Transform",
                    vec![("position".into(), ReflectValue::Vec3(glam::Vec3::ZERO))],
                ),
                component("RigidBody", vec![("kind".into(), ReflectValue::U32(0))]),
            ],
        ),
        named(
            1,
            "Hinge",
            vec![component(
                "Joint",
                vec![
                    ("kind".into(), ReflectValue::U32(1)),
                    (
                        "body_a".into(),
                        ReflectValue::EntityRef(Some(EntityRef::live(Entity::new(0, 0)))),
                    ),
                    ("body_b".into(), ReflectValue::EntityRef(None)),
                    ("stiffness".into(), ReflectValue::F32(100.0)),
                    ("breakable".into(), ReflectValue::Bool(false)),
                ],
            )],
        ),
    ]
}

/// Draws the Inspector `frames` times over unchanging data and returns
/// whatever egui complained about.
fn draw_repeatedly(
    entities: &[EntityDisplayInfo],
    selected: &[Entity],
    frames: usize,
) -> Vec<String> {
    install_logger();
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut euler_cache = std::collections::HashMap::new();
    let mut mode = RotationDisplayMode::Local;
    let complaints = drawing(frames, |ui, _| {
        let mut actions = Vec::new();
        draw_inspector_content(
            ui,
            entities,
            selected,
            &[],
            &mut actions,
            &mut euler_cache,
            &mut mode,
            &[],
            None,
            None,
        );
    });

    drop(guard);
    complaints
}

/// The reported case: one entity selected, its components on screen, and
/// nothing about it changing.
#[test]
fn a_selected_entity_keeps_its_widget_ids_across_frames() {
    let entities = scene();
    let selected = [Entity::new(1, 0)];
    let complaints = draw_repeatedly(&entities, &selected, 4);

    assert!(
        complaints.is_empty(),
        "the Inspector gave {} widget(s) a new id without the data changing:\n{}",
        complaints.len(),
        complaints.join("\n"),
    );
}

/// Selecting a different entity is not the bug — but redrawing *that*
/// selection repeatedly still has to be stable.
#[test]
fn a_second_selection_is_stable_too() {
    let entities = scene();
    let selected = [Entity::new(0, 0)];
    let complaints = draw_repeatedly(&entities, &selected, 4);

    assert!(
        complaints.is_empty(),
        "the Inspector gave {} widget(s) a new id without the data changing:\n{}",
        complaints.len(),
        complaints.join("\n"),
    );
}

/// The condition the reported log had and the tests above do not: the
/// selected entity is the *same* entity, drawn at the same place, holding
/// the same components — but its `Entity` handle changed.
///
/// That is what a mirror does when it despawns and respawns rather than
/// reusing a local entity. The Inspector keys its per-component id on
/// `entity.index()`, so every widget under it is renamed while the layout
/// stays put — a stable parent with a child whose id came from the data,
/// which is exactly the shape egui warns about.
#[test]
fn an_entity_whose_handle_changes_renames_every_widget() {
    install_logger();
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut euler_cache = std::collections::HashMap::new();
    let mut mode = RotationDisplayMode::Local;

    let complaints = drawing(4, |ui, frame| {
        // The same world, renumbered — one despawn/respawn cycle per frame.
        let generation = frame as u32;
        let body = Entity::new(0, generation);
        let hinge = Entity::new(1, generation);
        let entities = vec![
            EntityDisplayInfo {
                entity: body,
                ..named(0, "Door frame", vec![component("RigidBody", vec![])])
            },
            EntityDisplayInfo {
                entity: hinge,
                ..named(1, "Hinge", vec![component("Joint", vec![])])
            },
        ];
        let mut actions = Vec::new();
        draw_inspector_content(
            ui,
            &entities,
            &[hinge],
            &[],
            &mut actions,
            &mut euler_cache,
            &mut mode,
            &[],
            None,
            None,
        );
    });

    drop(guard);
    assert!(
        complaints.is_empty(),
        "a renumbered entity renamed {} widget(s):\n{}",
        complaints.len(),
        complaints.join("\n"),
    );
}
