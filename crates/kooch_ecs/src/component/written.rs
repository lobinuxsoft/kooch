//! Systems that react to writes (#1882): a quiet frame reads the change ticks and nothing else.

use kooch_core::change_tick::SystemTick;
use kooch_core::resource::Resources;

use crate::entity::Entity;

use super::{Component, ComponentRegistry};

/// The tick the running system last ran at; 0 outside a scheduled system, so it sees everything.
pub fn last_run(resources: &Resources) -> u64 {
    resources
        .get::<SystemTick>()
        .map_or(0, |tick| tick.last_run)
}

/// Whether any `T` was written after `since`.
pub fn written_since<T: Component>(registry: &ComponentRegistry, since: u64) -> bool {
    registry
        .get_cpu::<T>()
        .is_some_and(|view| view.iter_changed(since).next().is_some())
}

/// Runs `fix` on each `T` written since the running system last ran for which `stale` holds.
/// Only what it fixes is written, so a loader's one-off fix-up does not mark the whole storage
/// changed every frame.
pub fn fix_written<T: Component>(
    resources: &mut Resources,
    stale: impl Fn(&T) -> bool,
    mut fix: impl FnMut(Entity, &mut T),
) {
    let since = last_run(resources);
    let Some(registry) = resources.get_mut::<ComponentRegistry>() else {
        return;
    };
    let found: Vec<Entity> = match registry.get_cpu::<T>() {
        Some(view) => view
            .iter_changed(since)
            .filter(|(_, value)| stale(value))
            .map(|(&entity, _)| entity)
            .collect(),
        None => return,
    };
    if found.is_empty() {
        return;
    }
    let Some(mut view) = registry.get_cpu_mut::<T>() else {
        return;
    };
    for entity in found {
        if let Some(value) = view.get_mut(entity) {
            fix(entity, value);
        }
    }
}
