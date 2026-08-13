use kooch_remote::protocol::EntityId;

use super::*;

fn step(label: &str) -> Step {
    Step {
        label: label.to_owned(),
        inverse: Inverse::Despawn(vec![EntityId {
            index: 1,
            generation: 0,
        }]),
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
