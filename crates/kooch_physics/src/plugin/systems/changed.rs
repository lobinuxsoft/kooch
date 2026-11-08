//! Last sync's authored list, kept so a frame re-reads only the bodies written since (#1868, #1870).

use kooch_core::change_tick::SystemTick;
use kooch_ecs::hierarchy::{Children, GlobalTransform};
use kooch_ecs::post_process_volume::PostProcessVolume;

use super::*;

/// What the last sync read, and what it was read under.
#[derive(Default)]
pub(super) struct SyncCache {
    authored: Vec<Authored>,
    layers: Option<kooch_core::layers::LayerNames>,
    meshes: (u64, usize),
}

/// This frame's authored list: last frame's with the changed bodies re-read, or a full read when a
/// change cannot be pinned to a body — a removal, a region, the layer table, a mesh landing.
pub(super) fn authored(resources: &Resources, cache: &mut SyncCache) -> Option<Vec<Authored>> {
    let since = resources
        .get::<SystemTick>()
        .map_or(0, |tick| tick.last_run);
    let registry = resources.get::<ComponentRegistry>()?;
    let layers = layer_table(resources);
    let meshes = resources.get::<ColliderMeshCache>();
    let revision = meshes.map_or((0, 0), ColliderMeshCache::revision);

    let stale = cache.layers.as_ref() != Some(&layers)
        || cache.meshes != revision
        || registry.removed_at() > since
        || registry
            .get_cpu::<PostProcessVolume>()
            .is_some_and(|volumes| volumes.iter_changed(since).next().is_some());
    cache.layers = Some(layers.clone());
    cache.meshes = revision;
    let mut list = std::mem::take(&mut cache.authored);
    if stale || since == 0 {
        return read_authored(resources);
    }
    match reread(registry, &layers, meshes, since, &mut list) {
        Some(()) => Some(list),
        None => read_authored(resources),
    }
}

/// Hands the list back for the next frame.
pub(super) fn keep(cache: &mut SyncCache, authored: Vec<Authored>) {
    cache.authored = authored;
}

/// Re-reads, in place, every body written since `since`. `None` when a change belongs to something
/// the list holds that is no longer a body (a region), which only a full read rebuilds.
fn reread(
    registry: &ComponentRegistry,
    layers: &kooch_core::layers::LayerNames,
    meshes: Option<&ColliderMeshCache>,
    since: u64,
    list: &mut Vec<Authored>,
) -> Option<()> {
    let bodies = registry.get_cpu::<PhysicsBody>()?;
    let mut dirty: Vec<Entity> = bodies.iter_changed(since).map(|(e, _)| *e).collect();
    if let Some(colliders) = registry.get_cpu::<Collider>() {
        dirty.extend(colliders.iter_changed(since).map(|(e, _)| *e));
    }
    if let Some(globals) = registry.get_cpu::<GlobalTransform>() {
        dirty.extend(globals.iter_changed(since).map(|(e, _)| *e));
    }
    // A body that inherits shapes depends on its descendants: every parent is re-read.
    if let Some(children) = registry.get_cpu::<Children>() {
        dirty.extend(children.iter().map(|(e, _)| *e));
    }
    if dirty.is_empty() {
        return Some(());
    }
    let key = |entity: &Entity| (entity.index(), entity.generation());
    dirty.sort_unstable_by_key(key);
    dirty.dedup();

    let reader = Reader::new(registry, layers, meshes);
    for entity in dirty {
        let found = list.binary_search_by_key(&key(&entity), |a| key(&a.entity));
        match (found, bodies.get(entity)) {
            (Ok(at), Some(body)) => {
                let fresh = reader.body(entity, body);
                if let Some(slot) = list.get_mut(at) {
                    *slot = fresh;
                }
            }
            // Listed but not a body: a region moved or changed.
            (Ok(_), None) => return None,
            (Err(at), Some(body)) => list.insert(at, reader.body(entity, body)),
            (Err(_), None) => {}
        }
    }
    Some(())
}
