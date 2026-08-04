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
use super::processor::Processor;

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
    /// Three — a flying controller, where up is an input rather than
    /// gravity.
    Vector3,
}

/// The id a file written before [`Action::id`] existed deserialises to.
///
/// A real id is assigned by [`ActionMap::assign_missing_ids`] on load,
/// derived from the name so that it is the same on every load of the
/// same file. Random would mean a reference stored in a scene pointed at
/// nothing until someone opened the map and saved it.
fn unassigned_id() -> kooch_core::Guid {
    kooch_core::Guid::from_bytes([0; 16])
}

/// Derives a stable id from the map and action names.
///
/// FNV-1a over both, twice with different offsets to fill 16 bytes. Not
/// a cryptographic hash and does not need to be: it exists so a file
/// without ids reads the same way twice, and every id it produces is
/// replaced the first time the map is saved.
fn derived_id(map: &str, action: &str) -> kooch_core::Guid {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let hash = |seed: u64| -> u64 {
        let mut h = seed;
        for byte in map
            .bytes()
            .chain(b"/".iter().copied())
            .chain(action.bytes())
        {
            h ^= u64::from(byte);
            h = h.wrapping_mul(PRIME);
        }
        h
    };

    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&hash(OFFSET).to_le_bytes());
    bytes[8..].copy_from_slice(&hash(OFFSET ^ u64::MAX).to_le_bytes());
    kooch_core::Guid::from_bytes(bytes)
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
    /// Stable identity, written to the file and never reused.
    ///
    /// **This is what a reference points at**, not the name. Unity
    /// learned the same thing — `InputAction.id` exists so that
    /// "renaming the action does not break references" — and the engine
    /// learned it once already with assets, which is why a `.meta` holds
    /// a guid instead of trusting a filename.
    ///
    /// Without it the identifier of an action is its name, so every
    /// consumer has to spell that name out and a rename in the panel
    /// becomes a control that silently stops answering.
    ///
    /// Derived from the name when a file predates the field, rather than
    /// randomly: a random id would differ on every load until someone
    /// saved, so a reference stored in a scene would point at nothing
    /// until then. Derived, an old file is stable from the first load.
    #[serde(default = "unassigned_id")]
    pub id: kooch_core::Guid,
    /// What gameplay asks for. Unique within its map, and free to change:
    /// nothing refers to an action by it.
    pub name: String,
    /// What it produces, which decides how bindings are read.
    pub control_type: ControlType,
    /// Flat list; a composite is a head followed by its parts.
    pub bindings: Vec<Binding>,
    /// Applied to the **final value**, after the winning binding is
    /// chosen — so a normalize or a sensitivity is written once instead
    /// of on every binding.
    ///
    /// This is deliberately not Unity's arrangement. There the action's
    /// processors are applied to *each binding*, so a stick that already
    /// carries a deadzone from its layout gets a second one — a known
    /// source of "my stick feels wrong", questioned by a `////REVIEW` in
    /// their own `InputBinding.cs`. Applied once to the result there is
    /// nothing to double: a binding shapes the **device**, an action
    /// shapes the **meaning**.
    ///
    /// `#[serde(default)]` so every `.inputmap` written before this
    /// field existed still loads.
    #[serde(default)]
    pub processors: Vec<Processor>,
}

impl Action {
    /// An action with no bindings yet — what the editor's "add action"
    /// produces.
    pub fn new(name: impl Into<String>, control_type: ControlType) -> Self {
        Self {
            id: kooch_core::Guid::new_v4(),
            name: name.into(),
            control_type,
            bindings: Vec::new(),
            processors: Vec::new(),
        }
    }

    /// Gives this action an id derived from its name if it has none.
    ///
    /// `scope` is the map's name when it has one, so two maps can each
    /// hold a `jump` without colliding. A standalone `.inputaction`
    /// passes `""`.
    pub fn ensure_id(&mut self, scope: &str) {
        if self.id == unassigned_id() {
            self.id = derived_id(scope, &self.name);
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

    /// Gives every action without an id one derived from its name.
    ///
    /// Called on load. A file written before ids existed gets the same
    /// ones on every load, so a reference stored in a scene resolves
    /// immediately rather than only after someone opens and saves the
    /// map.
    pub fn assign_missing_ids(&mut self) {
        let map_name = self.name.clone();
        for action in &mut self.actions {
            action.ensure_id(&map_name);
        }
    }

    /// The index of the action with this id.
    ///
    /// What a stored reference resolves through — by identity, so a
    /// rename in the panel changes nothing here.
    pub fn resolve_ref(&self, id: kooch_core::Guid) -> Option<ActionId> {
        self.actions
            .iter()
            .position(|action| action.id == id)
            .map(|index| ActionId(index as u32))
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
}
