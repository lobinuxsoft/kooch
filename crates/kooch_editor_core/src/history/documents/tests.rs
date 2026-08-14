use std::path::PathBuf;

use kooch_core::resource::Resources;
use kooch_input::actions::{ActionMap, ControlType};

use crate::state::{OpenInputKind, OpenInputMap};

use super::*;

fn path() -> PathBuf {
    PathBuf::from("/project/assets/player.inputaction")
}

fn document() -> Document {
    Document::InputMap(path())
}

/// An editor with one input map open, holding one action.
fn editor() -> Resources {
    let mut resources = Resources::new();
    let mut map = ActionMap::new("Player");
    map.actions.push(kooch_input::actions::Action::new(
        "Jump",
        ControlType::Button,
    ));
    resources.insert(OpenInputMap {
        path: path(),
        kind: OpenInputKind::SingleAction,
        map,
        focus_requested: false,
        selected: None,
        dirty: false,
    });
    resources
}

fn rename(resources: &mut Resources, name: &str) {
    resources.get_mut::<OpenInputMap>().unwrap().map.actions[0].name = name.to_owned();
}

fn current_name(resources: &Resources) -> String {
    resources.get::<OpenInputMap>().unwrap().map.actions[0]
        .name
        .clone()
}

/// The round trip: record before an edit, undo puts the document back.
#[test]
fn an_undone_edit_comes_back() {
    let mut resources = editor();
    record(&mut resources, &document(), "Rename Action", None);
    rename(&mut resources, "Leap");
    assert_eq!(current_name(&resources), "Leap");

    assert!(step(&mut resources, &document(), true));
    assert_eq!(current_name(&resources), "Jump");
}

/// Redo is undo run the other way, and needs no code of its own — the
/// state being replaced always goes onto the opposite stack.
#[test]
fn a_redo_puts_it_forward() {
    let mut resources = editor();
    record(&mut resources, &document(), "Rename Action", None);
    rename(&mut resources, "Leap");

    assert!(step(&mut resources, &document(), true));
    assert!(step(&mut resources, &document(), false));
    assert_eq!(current_name(&resources), "Leap");
}

/// 🔴 The coalescing rule, at the level a user feels it: four keystrokes
/// are one step, and one Ctrl+Z gets the original name back rather than
/// three letters of it.
#[test]
fn typing_is_one_step() {
    let mut resources = editor();
    let key = Some(MergeKey::of(("rename", 0usize)));
    for letter in ["L", "Le", "Lea", "Leap"] {
        record(&mut resources, &document(), "Rename Action", key);
        rename(&mut resources, letter);
    }

    assert!(step(&mut resources, &document(), true));
    assert_eq!(current_name(&resources), "Jump");
    assert!(
        !step(&mut resources, &document(), true),
        "the run left more than one step behind",
    );
}

/// What the seal is for: the same field edited after a boundary is a
/// second step, so undoing it does not also undo the first.
#[test]
fn a_seal_splits_the_run() {
    let mut resources = editor();
    let key = Some(MergeKey::of(("rename", 0usize)));

    record(&mut resources, &document(), "Rename Action", key);
    rename(&mut resources, "Leap");
    resources.get_mut::<DocumentHistories>().unwrap().seal();
    record(&mut resources, &document(), "Rename Action", key);
    rename(&mut resources, "Vault");

    assert!(step(&mut resources, &document(), true));
    assert_eq!(current_name(&resources), "Leap", "the seal did not split");
    assert!(step(&mut resources, &document(), true));
    assert_eq!(current_name(&resources), "Jump");
}

/// Two documents, two histories: an undo aimed at one never reaches the
/// other. The whole point of routing by document.
#[test]
fn another_document_is_untouched() {
    let mut resources = editor();
    record(&mut resources, &document(), "Rename Action", None);
    rename(&mut resources, "Leap");

    let other = Document::InputMap(PathBuf::from("/project/assets/menu.inputaction"));
    assert!(
        !step(&mut resources, &other, true),
        "an undo reached a document that had no history",
    );
    assert_eq!(current_name(&resources), "Leap");
}

/// The scene's history lives elsewhere, and this module refuses to
/// pretend otherwise rather than silently doing nothing later.
#[test]
fn the_world_is_not_a_document() {
    let mut resources = editor();
    record(&mut resources, &Document::World, "Set position", None);
    assert!(!step(&mut resources, &Document::World, true));
}
