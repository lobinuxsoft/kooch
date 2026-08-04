//! [`ActionHandle`] — a name resolved once, kept until the map changes.
//!
//! # The problem it solves
//!
//! [`ActionMap::resolve`] is a linear scan comparing strings, and its own
//! doc says it is meant to be called once. A game that calls it per frame
//! pays that per action per frame, and reads as if the string *were* the
//! identity — which is exactly the habit [`ActionId`] exists to break.
//!
//! Caching the id by hand is worse than it looks, though: an `ActionId`
//! is an **index into one map**. Under a different map the same index
//! silently means another action, or is out of range. So a cached id is
//! only correct alongside a way to notice the map moved — which is what
//! [`ActiveActionMap::generation`] is for, and what this pairs with it.

use super::action::{ActionId, ActionMap};
use super::plugin::ActiveActionMap;

/// A resolved action, valid for as long as the map it came from.
///
/// ```ignore
/// let mut jump = ActionHandle::new("jump");
/// // per frame:
/// if let Some(id) = jump.id(&active) {
///     if state.just_pressed(id) { /* … */ }
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ActionHandle {
    name: String,
    /// `None` when the name is not in the current map — cached too, so a
    /// missing action costs one scan rather than one per frame.
    id: Option<ActionId>,
    /// The generation `id` was resolved under. `None` means never.
    resolved_at: Option<u32>,
}

impl ActionHandle {
    /// Declares the action this reads. Resolves nothing yet: at
    /// construction time there is usually no map loaded.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            id: None,
            resolved_at: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The id under `active`, resolving only when the map has changed.
    ///
    /// The steady-state cost is one `u32` compare. `None` means this map
    /// has no such action — renamed in the editor, or a map that was
    /// never meant to drive this system.
    pub fn id(&mut self, active: &ActiveActionMap) -> Option<ActionId> {
        if self.resolved_at != Some(active.generation) {
            self.id = active.map.resolve(&self.name);
            self.resolved_at = Some(active.generation);
        }
        self.id
    }

    /// Resolves against a bare map, for a caller holding no
    /// [`ActiveActionMap`] — a test, mostly.
    pub fn id_in(&mut self, map: &ActionMap) -> Option<ActionId> {
        self.id = map.resolve(&self.name);
        // Deliberately not recorded: this map has no generation, so the
        // next `id` call must not mistake this for a current answer.
        self.resolved_at = None;
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{Action, ControlType};

    fn map_with(names: &[&str]) -> ActionMap {
        names.iter().fold(ActionMap::new("m"), |map, name| {
            map.add(Action::new(*name, ControlType::Button))
        })
    }

    /// The point: the second frame does not scan.
    #[test]
    fn a_name_is_resolved_once_per_map() {
        let active = ActiveActionMap::new(map_with(&["move", "jump"]));
        let mut handle = ActionHandle::new("jump");

        assert_eq!(handle.id(&active).map(|id| id.index()), Some(1));
        assert_eq!(handle.resolved_at, Some(active.generation));
        // Same generation: the cached answer stands.
        assert_eq!(handle.id(&active).map(|id| id.index()), Some(1));
    }

    /// 🔴 An id is an index into **one** map. Kept across a swap it
    /// points at whatever now sits at that index — a jump that fires the
    /// action that took its place, which is worse than not firing.
    #[test]
    fn a_new_map_invalidates_the_cached_id() {
        let mut handle = ActionHandle::new("jump");

        let first = ActiveActionMap::new(map_with(&["move", "jump"]));
        assert_eq!(handle.id(&first).map(|id| id.index()), Some(1));

        // "jump" is now index 0, and index 1 is a different action.
        let mut second = ActiveActionMap::new(map_with(&["jump", "fire"]));
        second.generation = first.generation.wrapping_add(1);

        assert_eq!(
            handle.id(&second).map(|id| id.index()),
            Some(0),
            "the handle kept an index from the previous map"
        );
    }

    /// A name the map does not declare stays `None` without re-scanning
    /// for it every frame.
    #[test]
    fn a_missing_action_is_cached_as_missing() {
        let active = ActiveActionMap::new(map_with(&["move"]));
        let mut handle = ActionHandle::new("jump");

        assert_eq!(handle.id(&active), None);
        assert_eq!(
            handle.resolved_at,
            Some(active.generation),
            "a miss was not cached, so it rescans every frame"
        );
    }

    /// And it comes back when a map that declares it arrives.
    #[test]
    fn a_missing_action_resolves_once_the_map_declares_it() {
        let mut handle = ActionHandle::new("jump");
        let without = ActiveActionMap::new(map_with(&["move"]));
        assert_eq!(handle.id(&without), None);

        let mut with = ActiveActionMap::new(map_with(&["move", "jump"]));
        with.generation = without.generation.wrapping_add(1);
        assert_eq!(handle.id(&with).map(|id| id.index()), Some(1));
    }
}
