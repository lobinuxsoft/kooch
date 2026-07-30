//! The editor's side of prefab propagation: when to ask for it.
//!
//! The rule itself — which field of which entity takes which value, and
//! what an override protects — is engine logic and lives in
//! [`ome_ecs::scene::propagate`]. A scene has to catch up with its prefabs
//! the moment it *loads*, and the project is what loads it; leaving the
//! rule here would have meant waiting for the mirror before knowing what
//! to do.
//!
//! What is left here is the queue: a prefab saved while an action is being
//! handled cannot propagate inline, so it is noted and drained on the next
//! pass.

use ome_core::Guid;
use ome_core::resource::Resources;

pub(crate) use ome_ecs::scene::propagate::{
    PlannedRemoval, PlannedWrite, apply, plan, plan_revert, write_overrides,
};

/// Prefabs whose instances have not caught up with the file yet.
///
/// A set rather than a single guid: saving two prefabs in one frame has to
/// propagate both, and re-saving one before the drain has run must not
/// queue it twice.
#[derive(Default)]
pub(crate) struct PendingPropagation(std::collections::HashSet<Guid>);

impl PendingPropagation {
    pub(crate) fn queue(&mut self, prefab: Guid) {
        self.0.insert(prefab);
    }

    pub(crate) fn drain(&mut self) -> Vec<Guid> {
        self.0.drain().collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Whether any prefab is waiting to reach its instances.
///
/// Asked by the caller that skips action handling on idle frames.
/// Propagation is queued while an action is being handled and drained on
/// the next pass, so a queue that only drains when the user happens to do
/// something else is a queue that does not drain.
pub(crate) fn anything_queued(resources: &Resources) -> bool {
    resources
        .get::<PendingPropagation>()
        .is_some_and(|pending| !pending.is_empty())
}

/// Notes that `prefab` changed and its instances are behind.
pub(crate) fn queue(resources: &mut Resources, prefab: Guid) {
    tracing::info!(target: "ome_editor_core::prefab", %prefab, "queued for propagation");
    if resources.get::<PendingPropagation>().is_none() {
        resources.insert(PendingPropagation::default());
    }
    if let Some(pending) = resources.get_mut::<PendingPropagation>() {
        pending.queue(prefab);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saving one prefab twice before the drain runs must not propagate it
    /// twice, and draining has to leave the queue empty or the same work
    /// repeats every frame.
    #[test]
    fn the_queue_holds_each_prefab_once() {
        let mut pending = PendingPropagation::default();
        let prefab = Guid::new_v4();
        pending.queue(prefab);
        pending.queue(prefab);
        assert_eq!(pending.drain(), vec![prefab]);
        assert!(pending.drain().is_empty());
    }

    /// The idle-frame guard asks this. If it ever answered wrongly,
    /// propagation would go back to only happening when the user happened
    /// to do something else.
    #[test]
    fn a_queued_prefab_is_visible_to_the_frame_guard() {
        let mut resources = Resources::new();
        assert!(!anything_queued(&resources));
        queue(&mut resources, Guid::new_v4());
        assert!(anything_queued(&resources));
    }
}
