//! [`ActionRef`] — a field that points at an action, chosen in the editor.
//!
//! # What it replaces
//!
//! A game used to name the action it reads:
//!
//! ```ignore
//! map.resolve("jump")   // ← a string, in code, per frame
//! ```
//!
//! which makes the **name** the identity. Rename `jump` in the panel and
//! the control stops answering, with nothing to say so — the map made the
//! *bindings* data while leaving the action itself hard-coded.
//!
//! An `ActionRef` is a field on a component. It stores the action's
//! [`id`](super::action::Action::id) and is filled from a dropdown in the
//! Inspector, the same way a `MeshRenderer` is given a mesh. Gameplay
//! reads whatever it points at and never spells a name.
//!
//! Ported from Unity's `InputActionReference`, whose doc gives the same
//! reason: *"the reference will remain intact even if the action or the
//! map that contains the action is renamed"*.
//!
//! # Resolving it
//!
//! The id is an identity; reading a value needs an index into the map
//! that is active *now*. [`ActionRef::id`] does that lookup and caches
//! it against [`ActiveActionMap::generation`], so the steady-state cost
//! is a `u32` compare rather than a scan.

use kooch_core::Guid;

use super::action::ActionId;
use super::plugin::ActiveActionMap;

/// Points at one action, by identity.
///
/// Unset means "not configured yet", which reads as an action that never
/// fires — deliberately harmless, since a component added and not filled
/// in must not take the controls away from something else.
/// ⚠️ Not `Reflect` yet, so it cannot be a field the Inspector draws.
/// That needs a `FieldKind::ActionRef` — the picker has to list the
/// active map's actions, which is not what the asset picker does. The
/// model below is what such a field would store and how it resolves;
/// wiring it into reflection touches 35 match arms across scene
/// serialisation and the plugin bridge, and is its own change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActionRef {
    /// The action's own id, not its name and not its index.
    pub action: Option<Guid>,
}

impl ActionRef {
    pub fn new(action: Guid) -> Self {
        Self {
            action: Some(action),
        }
    }

    /// Whether this points at anything at all.
    pub fn is_set(&self) -> bool {
        self.action.is_some()
    }
}

/// An [`ActionRef`] plus the lookup it needs to be read per frame.
///
/// Kept separate from `ActionRef` so the component stays plain data: a
/// cache in a reflected field would be serialised into the scene and
/// would go stale there, which is the failure this whole file exists to
/// avoid one level up.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolvedAction {
    id: Option<ActionId>,
    resolved_at: Option<u32>,
}

impl ResolvedAction {
    /// The index `reference` points at under `active`.
    ///
    /// Re-resolves only when the map has been replaced. `None` means the
    /// reference is unset, or points at an action this map does not have
    /// — a scene configured against a different `.inputmap`.
    pub fn id(&mut self, reference: ActionRef, active: &ActiveActionMap) -> Option<ActionId> {
        if self.resolved_at != Some(active.generation) {
            self.id = reference.action.and_then(|id| active.map.resolve_ref(id));
            self.resolved_at = Some(active.generation);
        }
        self.id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::action::{Action, ActionMap, ControlType};

    fn map_with(names: &[&str]) -> ActionMap {
        names.iter().fold(ActionMap::new("m"), |map, name| {
            map.add(Action::new(*name, ControlType::Button))
        })
    }

    /// 🔴 The whole point: renaming an action does not break what points
    /// at it. This is what `resolve("jump")` could not do.
    #[test]
    fn a_reference_survives_a_rename() {
        let mut map = map_with(&["move", "jump"]);
        let reference = ActionRef::new(map.actions[1].id);
        let mut resolved = ResolvedAction::default();

        let active = ActiveActionMap::new(map.clone());
        assert_eq!(
            resolved.id(reference, &active).map(|id| id.index()),
            Some(1)
        );

        // Renamed in the panel. Same action, different name.
        map.actions[1].name = "leap".to_owned();
        let mut renamed = ActiveActionMap::new(map);
        renamed.generation = active.generation.wrapping_add(1);

        assert_eq!(
            resolved.id(reference, &renamed).map(|id| id.index()),
            Some(1),
            "the reference broke on a rename, which is the one thing it \
             exists to prevent"
        );
    }

    /// And it follows the action when the list is reordered — an index
    /// would not.
    #[test]
    fn a_reference_follows_its_action_when_the_list_moves() {
        let map = map_with(&["move", "jump"]);
        let reference = ActionRef::new(map.actions[1].id);
        let mut resolved = ResolvedAction::default();

        let mut reordered = ActionMap::new("m");
        reordered.actions = vec![map.actions[1].clone(), map.actions[0].clone()];
        let mut active = ActiveActionMap::new(reordered);
        active.generation = 7;

        assert_eq!(
            resolved.id(reference, &active).map(|id| id.index()),
            Some(0),
            "the reference kept a position instead of following identity"
        );
    }

    /// Unset is harmless: an action that never fires, not a panic and not
    /// somebody else's action.
    #[test]
    fn an_unset_reference_resolves_to_nothing() {
        let active = ActiveActionMap::new(map_with(&["move"]));
        let mut resolved = ResolvedAction::default();
        assert_eq!(resolved.id(ActionRef::default(), &active), None);
        assert!(!ActionRef::default().is_set());
    }

    /// Pointing at an action the active map does not have is also
    /// nothing — a scene configured against another `.inputmap`.
    #[test]
    fn a_reference_into_another_map_resolves_to_nothing() {
        let elsewhere = map_with(&["fire"]);
        let reference = ActionRef::new(elsewhere.actions[0].id);
        let active = ActiveActionMap::new(map_with(&["move", "jump"]));

        let mut resolved = ResolvedAction::default();
        assert_eq!(resolved.id(reference, &active), None);
    }

    /// The lookup happens once per map, not once per frame.
    #[test]
    fn resolution_is_cached_until_the_map_changes() {
        let map = map_with(&["move", "jump"]);
        let reference = ActionRef::new(map.actions[0].id);
        let active = ActiveActionMap::new(map);

        let mut resolved = ResolvedAction::default();
        assert_eq!(
            resolved.id(reference, &active).map(|id| id.index()),
            Some(0)
        );
        assert_eq!(resolved.resolved_at, Some(active.generation));
        assert_eq!(
            resolved.id(reference, &active).map(|id| id.index()),
            Some(0)
        );
    }
}
