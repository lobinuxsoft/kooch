use kooch_remote::protocol::EntityId;

use super::*;

fn step(label: &str) -> Step {
    Step {
        label: label.to_owned(),
        inverse: Inverse::Despawn(vec![EntityId {
            index: 1,
            generation: 0,
        }]),
        key: None,
    }
}

/// Nothing to undo until something is recorded.
#[test]
fn an_empty_history_offers_nothing() {
    let history = RemoteHistory::default();
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(history.undo_description(), None);
}

/// The Edit menu names the step it would take, so the label has to
/// survive the recording.
#[test]
fn the_next_step_is_named() {
    let mut history = RemoteHistory::default();
    history.record(step("Duplicate Entity"));
    assert!(history.can_undo());
    assert_eq!(history.undo_description(), Some("Duplicate Entity"));
}

/// 🔴 Editing after an undo drops the redo branch. Keeping it would
/// offer to redo an edit against a world that has since moved somewhere
/// else — the classic way an undo history corrupts what it was
/// protecting.
#[test]
fn a_new_edit_drops_the_redo() {
    let mut history = RemoteHistory::default();
    history.record(step("Set position"));
    // Standing in for an undo, which is what fills the redo stack.
    history.undone.push(step("Set position"));
    assert!(history.can_redo());

    history.record(step("Despawn Entity"));
    assert!(!history.can_redo(), "the redo branch outlived the edit");
}

/// The history is bounded: an editor left open for a day holds a
/// hundred steps, not a day of them.
#[test]
fn the_history_stops_growing() {
    let mut history = RemoteHistory::default();
    for i in 0..DEPTH + 10 {
        history.record(step(&format!("Edit {i}")));
    }
    assert_eq!(history.done.len(), DEPTH);
    // The oldest went, not the newest.
    assert_eq!(
        history.undo_description(),
        Some(format!("Edit {}", DEPTH + 9).as_str()),
    );
}

/// Closing a project takes its history with it: the ids in it name
/// entities in a world that is gone.
#[test]
fn clearing_empties_both_stacks() {
    let mut history = RemoteHistory::default();
    history.record(step("Spawn Entity"));
    history.undone.push(step("Despawn Entity"));

    history.clear();
    assert!(!history.can_undo());
    assert!(!history.can_redo());
}

/// The words in the menu come from the action, and they are the same
/// words the local commands use.
#[test]
fn a_step_is_labelled_by_action() {
    assert_eq!(label_of(&EditorAction::PasteEntities), "Paste");
    assert_eq!(
        label_of(&EditorAction::Duplicate(kooch_ecs::entity::Entity::new(
            1, 0
        ))),
        "Duplicate Entity",
    );
    assert_eq!(
        label_of(&EditorAction::TransformEdit {
            entity: kooch_ecs::entity::Entity::new(1, 0),
            before: kooch_ecs::transform::Transform::default(),
            after: kooch_ecs::transform::Transform::default(),
            desc: "Move Entity",
        }),
        "Move Entity",
    );
}

fn field_step(before: f32, after: f32, key: Option<crate::history::MergeKey>) -> Step {
    Step {
        label: "Set intensity".to_owned(),
        inverse: Inverse::SetField {
            entity: EntityId {
                index: 1,
                generation: 0,
            },
            component: "Light".to_owned(),
            field: "intensity".to_owned(),
            before: kooch_ecs::reflect::ReflectValue::F32(before),
            after: kooch_ecs::reflect::ReflectValue::F32(after),
        },
        key,
    }
}

fn intensity(history: &RemoteHistory) -> (f32, f32) {
    match &history.done.last().expect("no step").inverse {
        Inverse::SetField {
            before: kooch_ecs::reflect::ReflectValue::F32(before),
            after: kooch_ecs::reflect::ReflectValue::F32(after),
            ..
        } => (*before, *after),
        _ => panic!("not a field step"),
    }
}

/// 🔴 The Inspector emits an edit per frame of a drag. Sixty of them are
/// one step, holding where the drag started and where it ended — the
/// difference between one Ctrl+Z and sixty.
#[test]
fn a_drag_is_one_step() {
    let mut history = RemoteHistory::default();
    let key = Some(crate::history::MergeKey::of("intensity"));
    for frame in 1..=60 {
        history.record(field_step(0.0, frame as f32, key));
    }

    assert_eq!(history.done.len(), 1, "the drag filed more than one step");
    assert_eq!(
        intensity(&history),
        (0.0, 60.0),
        "the merged step lost an end of the drag",
    );
}

/// A boundary between two drags of the same field keeps them apart, or
/// undoing the second would silently undo the first as well.
#[test]
fn a_seal_splits_two_drags() {
    let mut history = RemoteHistory::default();
    let key = Some(crate::history::MergeKey::of("intensity"));
    history.record(field_step(0.0, 5.0, key));
    history.seal();
    history.record(field_step(5.0, 9.0, key));

    assert_eq!(history.done.len(), 2);
    assert_eq!(intensity(&history), (5.0, 9.0));
}

/// A step with no key is discrete however fast it arrives.
#[test]
fn keyless_steps_stay_apart() {
    let mut history = RemoteHistory::default();
    history.record(field_step(0.0, 1.0, None));
    history.record(field_step(1.0, 2.0, None));
    assert_eq!(history.done.len(), 2);
}
