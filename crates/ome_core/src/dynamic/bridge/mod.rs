//! Bridge between the C ABI [`EngineApi`] and the engine internals.
//!
//! Each `extern "C"` function here implements one slot of the
//! [`EngineApi`] function pointer table. They cast the opaque `ctx`
//! back to a [`BridgeContext`] and forward calls to real engine types.

use std::ffi::c_void;

use ome_plugin_api::engine_api::EngineApi;

mod callback;
mod context;
mod entity;
mod resource_api;
#[cfg(test)]
mod tests;

pub use context::BridgeContext;
pub use entity::EntityBridge;

/// Constructs an [`EngineApi`] wired to the given [`BridgeContext`].
pub fn create_engine_api(ctx: &mut BridgeContext) -> EngineApi {
    EngineApi {
        ctx: ctx as *mut BridgeContext as *mut c_void,
        spawn_entity: entity::bridge_spawn_entity,
        despawn_entity: entity::bridge_despawn_entity,
        get_resource_ptr: resource_api::bridge_get_resource_ptr,
        get_resource_mut_ptr: resource_api::bridge_get_resource_mut_ptr,
        add_system: callback::bridge_add_system,
        log_info: resource_api::bridge_log_info,
        set_plugin_data: resource_api::bridge_set_plugin_data,
        get_plugin_data: resource_api::bridge_get_plugin_data,
    }
}
