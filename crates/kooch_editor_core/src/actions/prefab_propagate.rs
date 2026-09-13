//! The editor's side of prefab propagation: when to ask for it.

use kooch_core::Guid;
use kooch_core::resource::Resources;

pub(crate) use kooch_ecs::scene::propagate::{
    PlannedRemoval, PlannedWrite, apply, plan, plan_revert, write_overrides,
};

/// Prefabs whose instances have not caught up with the file yet.
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
pub(crate) fn anything_queued(resources: &Resources) -> bool {
    let propagation = resources
        .get::<PendingPropagation>()
        .is_some_and(|pending| !pending.is_empty());
    // The reload notice rides the same drain, so the guard has to ask
    // about it too — the last time one of these was left out, the queue
    // only emptied when the user happened to do something else.
    let reloads = resources
        .get::<crate::actions::handlers::PendingHostReloads>()
        .is_some_and(|pending| !pending.0.is_empty());
    propagation || reloads
}

/// Notes that `prefab` changed and its instances are behind.
pub(crate) fn queue(resources: &mut Resources, prefab: Guid) {
    tracing::info!(target: "kooch_editor_core::prefab", %prefab, "queued for propagation");
    if resources.get::<PendingPropagation>().is_none() {
        resources.insert(PendingPropagation::default());
    }
    if let Some(pending) = resources.get_mut::<PendingPropagation>() {
        pending.queue(prefab);
    }
}

#[cfg(test)]
mod tests;
