use super::*;
use crate::panels::input_map::{BindingAddress, InputMapAction as Edit};
use crate::state::OpenInputMap;
use kooch_input::actions::{Action, ActionMap, Binding, ControlPath, ControlType};
use kooch_input::ids::{GamepadButton, KeyCode};

fn open(path: std::path::PathBuf) -> OpenInputMap {
    OpenInputMap {
        path,
        kind: crate::state::OpenInputKind::Map,
        map: ActionMap::new("gameplay").add(
            Action::new("jump", ControlType::Button)
                .bind(Binding::to(ControlPath::Key(KeyCode::Space))),
        ),
        focus_requested: false,
        selected: None,
        dirty: false,
    }
}

fn resources_with(open_map: OpenInputMap) -> Resources {
    let mut resources = Resources::new();
    resources.insert(open_map);
    resources
}

/// The contract the prefab already has: an edit changes the document
/// and **not** the file.
#[test]
fn an_edit_does_not_touch_the_file() {
    let dir = std::env::temp_dir().join("kooch_inputmap_edit_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Gameplay.inputmap");
    std::fs::write(&path, "original contents").unwrap();

    let mut resources = resources_with(open(path.clone()));
    edit_input_map(&mut resources, &Edit::AddAction);

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "original contents",
        "the edit reached the file before anyone asked it to"
    );
    let open = resources.get::<OpenInputMap>().unwrap();
    assert_eq!(
        open.map.actions.len(),
        2,
        "the edit did not reach the document"
    );
    assert!(open.dirty, "an edit left the map looking saved");

    let _ = std::fs::remove_dir_all(&dir);
}

/// And saving is what writes, after which nothing is outstanding.
#[test]
fn saving_writes_the_file_and_clears_the_marker() {
    let dir = std::env::temp_dir().join("kooch_inputaction_save_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Jump.inputaction");

    let action = kooch_input::actions::Action::new("jump", ControlType::Button);
    kooch_input::actions::save_action(&action, &path).expect("write");

    let mut resources = Resources::new();
    open_input_map(&mut resources, &path);
    edit_input_map(
        &mut resources,
        &Edit::RenameAction {
            action: 0,
            name: "leap".into(),
        },
    );
    assert!(resources.get::<OpenInputMap>().unwrap().dirty);
    save_input_map(&mut resources);

    let written = std::fs::read_to_string(&path).unwrap();
    assert!(
        written.contains("leap"),
        "the edit was not saved: {written}"
    );
    assert!(
        !resources.get::<OpenInputMap>().unwrap().dirty,
        "the file was written and the document still claims an unsaved change"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A rebind changes what a binding reads and nothing else — its
/// processors and its part in a composite belong to the binding, not
/// to the control behind it.
#[test]
fn a_rebind_replaces_only_the_control() {
    let mut resources = resources_with(open("unused.inputmap".into()));
    edit_input_map(
        &mut resources,
        &Edit::Rebind {
            at: BindingAddress {
                action: 0,
                binding: 0,
            },
            path: ControlPath::Button(GamepadButton::South),
        },
    );
    let open = resources.get::<OpenInputMap>().unwrap();
    let binding = &open.map.actions[0].bindings[0];
    assert_eq!(
        binding.path(),
        Some(ControlPath::Button(GamepadButton::South))
    );
    assert!(binding.processors.is_empty());
}

/// Two actions of one name make `resolve` a coin toss, so adding one
/// has to pick a name nobody is using.
#[test]
fn a_new_action_does_not_collide() {
    let mut resources = resources_with(open("unused.inputmap".into()));
    for _ in 0..3 {
        edit_input_map(&mut resources, &Edit::AddAction);
    }
    let open = resources.get::<OpenInputMap>().unwrap();
    assert!(
        open.map.duplicate_names().is_empty(),
        "adding actions produced duplicates: {:?}",
        open.map.duplicate_names()
    );
}

/// An index out of range is a stale click — the panel drew a list one
/// frame and the document changed. It must not panic the editor.
#[test]
fn an_edit_aimed_past_the_end_is_ignored() {
    let mut resources = resources_with(open("unused.inputmap".into()));
    for edit in [
        Edit::RemoveAction { action: 99 },
        Edit::AddBinding { action: 99 },
        Edit::RemoveBinding(BindingAddress {
            action: 0,
            binding: 99,
        }),
    ] {
        edit_input_map(&mut resources, &edit);
    }
    let open = resources.get::<OpenInputMap>().unwrap();
    assert_eq!(open.map.actions.len(), 1);
    assert!(!open.dirty, "a no-op edit marked the map unsaved");
}
