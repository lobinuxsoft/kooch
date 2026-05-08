// ---------------------------------------------------------------------------
// Entity bridge — delegates to EntityBridge resource (registered by ome_ecs)
// ---------------------------------------------------------------------------

use std::ffi::c_void;

use crate::resource::Resources;

use super::context::BridgeContext;

/// Trait-free entity operations registered by the ECS crate.
///
/// Stored as a resource so `ome_core` doesn't depend on `ome_ecs` types.
/// The closures capture `EntityAllocator` access internally.
pub struct EntityBridge {
    spawn_fn: Box<dyn Fn(&mut Resources) -> u64 + Send + Sync>,
    despawn_fn: Box<dyn Fn(&mut Resources, u64) -> bool + Send + Sync>,
}

impl EntityBridge {
    /// Creates a new entity bridge with custom spawn/despawn logic.
    pub fn new(
        spawn: impl Fn(&mut Resources) -> u64 + Send + Sync + 'static,
        despawn: impl Fn(&mut Resources, u64) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            spawn_fn: Box::new(spawn),
            despawn_fn: Box::new(despawn),
        }
    }
}

pub(super) extern "C" fn bridge_spawn_entity(ctx: *mut c_void) -> u64 {
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };
    let resources = unsafe { &mut *bridge.resources };

    // Remove-use-reinsert to avoid aliasing (EntityBridge borrows from Resources).
    if let Some(entity_bridge) = resources.remove::<EntityBridge>() {
        let result = (entity_bridge.spawn_fn)(resources);
        resources.insert(entity_bridge);
        result
    } else {
        tracing::warn!("spawn_entity called but no EntityBridge registered");
        0
    }
}

pub(super) extern "C" fn bridge_despawn_entity(ctx: *mut c_void, entity: u64) -> u32 {
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };
    let resources = unsafe { &mut *bridge.resources };

    if let Some(entity_bridge) = resources.remove::<EntityBridge>() {
        let result = (entity_bridge.despawn_fn)(resources, entity);
        resources.insert(entity_bridge);
        u32::from(result)
    } else {
        tracing::warn!("despawn_entity called but no EntityBridge registered");
        0
    }
}
