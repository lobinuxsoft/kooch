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

/// Selecting a different entity: the reported case, at last.
///
/// The Inspector keys each component's collapsing state on the selected
/// entity's *index*, so moving the selection renames every widget under it
/// while the layout stays exactly where it was — two entities with the
/// same components lay out identically. A stable parent with children
/// whose ids came from the data, which is the shape egui warns about.
///
/// The earlier version of this test varied the entity's *generation* and
/// found nothing, because the id is built from `index()` alone.
#[test]
fn moving_the_selection_keeps_the_widget_ids() {
    install_logger();
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let mut euler_cache = std::collections::HashMap::new();
    let mut mode = RotationDisplayMode::Local;

    // Two entities carrying the same components, so the layout is identical
    // and only the selection moves.
    let entities = vec![
        named(0, "Crate A", vec![component("RigidBody", vec![])]),
        named(1, "Crate B", vec![component("RigidBody", vec![])]),
    ];

    let complaints = drawing(4, |ui, frame| {
        let selected = [entities[frame % 2].entity];
        let mut actions = Vec::new();
        draw_inspector_content(
            ui,
            &entities,
            &selected,
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
        "moving the selection renamed {} widget(s):\n{}",
        complaints.len(),
        complaints.join("\n"),
    );
}
