use super::*;
use crate::actions::binding::{Binding, Composite, PartName, VectorMode};
use crate::actions::path::ControlPath;
use crate::ids::{GamepadButton, KeyCode};

fn gameplay() -> ActionMap {
    ActionMap::new("gameplay")
        .add(Action::new("move", ControlType::Vector2).bind_all([
            Binding::composite(Composite::Vector2 {
                mode: VectorMode::DigitalNormalized,
            }),
            Binding::part(PartName::Up, ControlPath::Key(KeyCode::KeyW)),
            Binding::part(PartName::Down, ControlPath::Key(KeyCode::KeyS)),
            Binding::part(PartName::Left, ControlPath::Key(KeyCode::KeyA)),
            Binding::part(PartName::Right, ControlPath::Key(KeyCode::KeyD)),
        ]))
        .add(
            Action::new("jump", ControlType::Button)
                .bind(Binding::to(ControlPath::Key(KeyCode::Space)))
                .bind(Binding::to(ControlPath::Button(GamepadButton::South))),
        )
}

/// The thing `ActionMap<A>` could not do, and the reason for all of
/// this: the whole model has to survive being written to a file.
#[test]
fn a_map_round_trips_through_the_engines_own_format() {
    let map = gameplay();
    let encoded = ron::to_string(&map).expect("serialise");
    let decoded: ActionMap = ron::from_str(&encoded).expect("deserialise");
    assert_eq!(decoded, map);
}

/// Gameplay holds an id, not a string.
#[test]
fn a_name_resolves_to_a_stable_id() {
    let map = gameplay();
    let jump = map.resolve("jump").expect("jump exists");
    assert_eq!(map.action(jump).map(|a| a.name.as_str()), Some("jump"));
    assert_eq!(map.resolve("jump"), Some(jump), "the id moved");
    assert_eq!(map.resolve("fly"), None, "an unknown name must not resolve");
}

/// One action, two devices, no branching in gameplay — the thing
/// roll-a-ball had to write by hand per device.
#[test]
fn one_action_takes_bindings_from_several_devices() {
    let map = gameplay();
    let jump = map.action(map.resolve("jump").unwrap()).unwrap();
    let devices: Vec<_> = jump
        .bindings
        .iter()
        .filter_map(|b| b.path().map(|p| p.device()))
        .collect();
    assert_eq!(devices.len(), 2);
    assert_ne!(devices[0], devices[1], "both bindings are the same device");
}

/// Two actions with one name make `resolve` a coin toss, so it has to
/// be answerable before a file is saved rather than after.
#[test]
fn duplicate_names_are_reported() {
    let map = ActionMap::new("gameplay")
        .add(Action::new("jump", ControlType::Button))
        .add(Action::new("jump", ControlType::Button))
        .add(Action::new("move", ControlType::Vector2));
    assert_eq!(map.duplicate_names(), vec!["jump"]);
    assert!(gameplay().duplicate_names().is_empty());
}
