//! Does the Inspector hand the same widget the same id every frame?

use crate::state::ReflectedFields;
use kooch_ecs::component::ComponentId;
use kooch_ecs::entity::Entity;
use kooch_ecs::reflect::{EntityRef, InspectorVisibility, ReflectValue};

use super::{RotationDisplayMode, draw_inspector_content};
use crate::panels::id_stability_probe::{drawing, install_logger};
use crate::state::{ComponentDisplayInfo, EntityDisplayInfo};

fn component(name: &str, fields: Vec<(String, ReflectValue)>) -> ComponentDisplayInfo {
    ComponentDisplayInfo {
        type_id: std::any::TypeId::of::<()>(),
        component: ComponentId(name.len() as u32),
        short_name: name.to_owned().into(),
        fields: ReflectedFields::Values(fields),
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
        is_prefab_instance: false,
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
                component("PhysicsBody", vec![("kind".into(), ReflectValue::U32(0))]),
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
    let guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let mut euler_cache = std::collections::HashMap::new();
    let mut mode = RotationDisplayMode::Local;
    let complaints = drawing(frames, |ui, _| {
        let mut actions = Vec::new();
        let labels = kooch_core::layers::LayerNames::default().labels();
        draw_inspector_content(
            ui,
            false,
            &mut Default::default(),
            entities,
            selected,
            &[],
            &mut actions,
            &mut euler_cache,
            &mut mode,
            &[],
            None,
            None,
            &labels,
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
#[test]
fn moving_the_selection_keeps_the_widget_ids() {
    install_logger();
    let guard = crate::panels::id_stability_probe::PROBE_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let mut euler_cache = std::collections::HashMap::new();
    let mut mode = RotationDisplayMode::Local;

    // Two entities carrying the same components, so the layout is identical and only the selection
    // moves.
    let body = || {
        component(
            "PhysicsBody",
            vec![
                ("mass".into(), ReflectValue::F32(1.0)),
                ("kind".into(), ReflectValue::U32(0)),
                ("ccd".into(), ReflectValue::Bool(false)),
            ],
        )
    };
    let entities = vec![
        named(0, "Crate A", vec![body()]),
        named(1, "Crate B", vec![body()]),
    ];

    let complaints = drawing(4, |ui, frame| {
        let selected = [entities[frame % 2].entity];
        let mut actions = Vec::new();
        let labels = kooch_core::layers::LayerNames::default().labels();
        draw_inspector_content(
            ui,
            false,
            &mut Default::default(),
            &entities,
            &selected,
            &[],
            &mut actions,
            &mut euler_cache,
            &mut mode,
            &[],
            None,
            None,
            &labels,
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
