//! [`Action`] and [`ActionMap`] — the data an editor authors.
//!
//! # An action is a name, not a type
//!
//! `ActionMap<A: Action>` keyed its bindings by a Rust enum. A type does
//! not serialise, does not appear in an Inspector and cannot be edited,
//! so authoring bindings in a panel was impossible by construction —
//! which is why #58 was blocked on this rather than on drawing widgets.
//!
//! Here an action is a **string name** and a control type. Unity reached
//! the same place; so did Rewired before it.
//!
//! ⚠️ The cost is real and worth naming: a typo in a name is a lookup
//! that silently finds nothing, where an enum would not have compiled.
//! That is bought back at the edges — [`ActionMap::resolve`] hands out a
//! stable [`ActionId`] once, and gameplay holds the id rather than
//! re-looking-up a string every frame.
//!
//! # Priority, and why maps stack
//!
//! Unity's action maps are switched on and off by hand. Unreal's mapping
//! contexts **stack with a priority** and consume what they handle, so
//! "in vehicle" sitting over "on foot" is the engine's job and not every
//! game's re-implementation. It costs one field and changes nothing about
//! the panel, so it is taken from Unreal rather than Unity.

use serde::{Deserialize, Serialize};

use super::binding::Binding;

/// What an action produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ControlType {
    /// On or off, with an edge — jump, fire, confirm.
    #[default]
    Button,
    /// One number — a trigger, a throttle.
    Axis,
    /// Two — a stick, WASD, a d-pad.
    Vector2,
}

/// A stable handle to an action inside its map.
///
/// Resolved once from a name; gameplay keeps this. An index rather than a
/// string because the lookup happens per action per frame, and because
/// this is the shape the rest of the engine already uses for identity —
/// entities, physics slots, meshlets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActionId(pub u32);

impl ActionId {
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// One thing the player can do, and everything that triggers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    /// What gameplay asks for. Unique within its map.
    pub name: String,
    /// What it produces, which decides how bindings are read.
    pub control_type: ControlType,
    /// Flat list; a composite is a head followed by its parts.
    pub bindings: Vec<Binding>,
}

impl Action {
    /// An action with no bindings yet — what the editor's "add action"
    /// produces.
    pub fn new(name: impl Into<String>, control_type: ControlType) -> Self {
        Self {
            name: name.into(),
            control_type,
            bindings: Vec::new(),
        }
    }

    /// Adds a binding, returning self so a map reads as a declaration.
    pub fn bind(mut self, binding: Binding) -> Self {
        self.bindings.push(binding);
        self
    }

    /// Adds several — a composite head and its parts, usually.
    pub fn bind_all(mut self, bindings: impl IntoIterator<Item = Binding>) -> Self {
        self.bindings.extend(bindings);
        self
    }
}

/// A named group of actions that can be pushed over another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionMap {
    pub name: String,
    /// Higher wins. A map on top **consumes** the actions it declares, so
    /// a lower map does not also see them.
    ///
    /// The point is that "driving" and "on foot" both binding `South` is
    /// not a conflict to resolve in gameplay: push the vehicle map and
    /// the on-foot jump stops answering, without either map knowing the
    /// other exists.
    pub priority: i32,
    pub actions: Vec<Action>,
}

impl ActionMap {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            priority: 0,
            actions: Vec::new(),
        }
    }

    /// Sets the priority. Higher sits on top.
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn add(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    /// The id for a name, or `None` if this map has no such action.
    ///
    /// Called once at startup. A game that calls it per frame is paying
    /// for a string compare per action per frame, which is exactly what
    /// [`ActionId`] exists to avoid.
    pub fn resolve(&self, name: &str) -> Option<ActionId> {
        self.actions
            .iter()
            .position(|action| action.name == name)
            .map(|index| ActionId(index as u32))
    }

    pub fn action(&self, id: ActionId) -> Option<&Action> {
        self.actions.get(id.index())
    }

    /// Names that appear more than once.
    ///
    /// Two actions of the same name make [`resolve`](Self::resolve) a
    /// coin toss, and the editor should refuse to save one — so this
    /// exists to be asked, rather than discovered at runtime.
    pub fn duplicate_names(&self) -> Vec<&str> {
        let mut seen: Vec<&str> = Vec::new();
        let mut duplicates: Vec<&str> = Vec::new();
        for action in &self.actions {
            let name = action.name.as_str();
            if seen.contains(&name) {
                if !duplicates.contains(&name) {
                    duplicates.push(name);
                }
            } else {
                seen.push(name);
            }
        }
        duplicates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::binding::{Binding, Composite, PartName, Vector2Mode};
    use crate::actions::path::ControlPath;
    use crate::ids::{GamepadButton, KeyCode};

    fn gameplay() -> ActionMap {
        ActionMap::new("gameplay")
            .add(Action::new("move", ControlType::Vector2).bind_all([
                Binding::composite(Composite::Vector2 {
                    mode: Vector2Mode::DigitalNormalized,
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
}
