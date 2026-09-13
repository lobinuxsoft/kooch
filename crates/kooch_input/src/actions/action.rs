//! [`Action`] and [`ActionMap`] — authored data: an action is a name and a control type, and
//! gameplay holds a resolved [`ActionId`], not a string.
//! Maps stack by priority and consume what they handle, as Unreal's mapping contexts do.

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

/// The id a file written before [`Action::id`] existed deserialises to, replaced on load by
/// [`ActionMap::assign_missing_ids`] with one derived from the name, stable across loads.
fn unassigned_id() -> kooch_core::Guid {
    kooch_core::Guid::from_bytes([0; 16])
}

/// Derives a stable id from the map and action names: FNV-1a twice for 16 bytes. Not cryptographic
/// — it only has to read the same twice until the map is saved.
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

/// A stable handle to an action inside its map, resolved once from a name — an index, as the engine
/// names entities and slots.
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
    /// Stable identity, written to the file and never reused — **what a reference points at**, not
    /// the name, so a rename does not break references.
    /// Derived from the name for older files, stable from the first load.
    #[serde(default = "unassigned_id")]
    pub id: kooch_core::Guid,
    /// What gameplay asks for. Unique within its map, and free to change:
    /// nothing refers to an action by it.
    pub name: String,
    /// What it produces, which decides how bindings are read.
    pub control_type: ControlType,
    /// Flat list; a composite is a head followed by its parts.
    pub bindings: Vec<Binding>,
    /// Applied to the **final value**, after the winning binding is chosen, so a normalize or
    /// sensitivity is written once — not per binding, which is how Unity doubles a deadzone.
    /// Defaults for older files.
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

    /// Gives this action a name-derived id if it has none; `scope` is the map's name, so two maps
    /// can each hold a `jump`, and empty for a standalone action.
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
    /// Higher wins, and a map on top **consumes** the actions it declares — push the vehicle map
    /// and the on-foot jump stops answering, with neither knowing the other.
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

    pub fn add(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    /// Gives every action without an id one derived from its name, on load, so references resolve
    /// before anyone saves the map.
    pub fn assign_missing_ids(&mut self) {
        let map_name = self.name.clone();
        for action in &mut self.actions {
            action.ensure_id(&map_name);
        }
    }

    /// The id for a name, or `None`. Call once at startup; per frame it is the string compare
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

    /// Names that appear more than once, which make [`resolve`](Self::resolve) a coin toss — for
    /// the editor to refuse saving.
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
mod tests;
