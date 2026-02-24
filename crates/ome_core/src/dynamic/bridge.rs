//! Bridge between the C ABI [`EngineApi`] and the engine internals.
//!
//! Each `extern "C"` function here implements one slot of the
//! [`EngineApi`] function pointer table. They cast the opaque `ctx`
//! back to a [`BridgeContext`] and forward calls to real engine types.

use std::ffi::c_void;

use ome_plugin_api::engine_api::EngineApi;
use ome_plugin_api::types::{SystemCallback, UserdataDrop};

use crate::resource::Resources;
use crate::schedule::Schedule;
use crate::stage::Stage;

use super::plugin_data::PluginData;
use super::resource_registry::ResourceRegistry;

/// Engine-side context behind the opaque `EngineApi::ctx` pointer.
///
/// Created on the stack for each plugin interaction:
/// - During `OmePlugin::build()`: both `resources` and `schedule` are set
/// - During system callbacks: only `resources` is set (`schedule` is null)
pub struct BridgeContext {
    pub resources: *mut Resources,
    pub schedule: *mut Schedule,
}

/// Constructs an [`EngineApi`] wired to the given [`BridgeContext`].
pub fn create_engine_api(ctx: &mut BridgeContext) -> EngineApi {
    EngineApi {
        ctx: ctx as *mut BridgeContext as *mut c_void,
        spawn_entity: bridge_spawn_entity,
        despawn_entity: bridge_despawn_entity,
        get_resource_ptr: bridge_get_resource_ptr,
        get_resource_mut_ptr: bridge_get_resource_mut_ptr,
        add_system: bridge_add_system,
        log_info: bridge_log_info,
        set_plugin_data: bridge_set_plugin_data,
        get_plugin_data: bridge_get_plugin_data,
    }
}

// ---------------------------------------------------------------------------
// Entity bridge — delegates to EntityBridge resource (registered by ome_ecs)
// ---------------------------------------------------------------------------

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

extern "C" fn bridge_spawn_entity(ctx: *mut c_void) -> u64 {
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

extern "C" fn bridge_despawn_entity(ctx: *mut c_void, entity: u64) -> u32 {
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

// ---------------------------------------------------------------------------
// Resource access bridge
// ---------------------------------------------------------------------------

extern "C" fn bridge_get_resource_ptr(
    ctx: *mut c_void,
    name_ptr: *const u8,
    name_len: usize,
) -> *const c_void {
    let bridge = unsafe { &*(ctx as *const BridgeContext) };
    let resources = unsafe { &*bridge.resources };
    let name = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len)) };

    let type_id = {
        let registry = match resources.get::<ResourceRegistry>() {
            Some(r) => r,
            None => return std::ptr::null(),
        };
        match registry.get_type_id(name) {
            Some(id) => id,
            None => return std::ptr::null(),
        }
    };

    resources.get_ptr_by_id(type_id) as *const c_void
}

extern "C" fn bridge_get_resource_mut_ptr(
    ctx: *mut c_void,
    name_ptr: *const u8,
    name_len: usize,
) -> *mut c_void {
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };
    let resources = unsafe { &mut *bridge.resources };
    let name = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len)) };

    // Two-step: lookup TypeId (immutable), then get pointer (mutable).
    // The TypeId copy breaks the immutable borrow before the mutable one.
    let type_id = {
        let registry = match resources.get::<ResourceRegistry>() {
            Some(r) => r,
            None => return std::ptr::null_mut(),
        };
        match registry.get_type_id(name) {
            Some(id) => id,
            None => return std::ptr::null_mut(),
        }
    };

    resources.get_mut_ptr_by_id(type_id) as *mut c_void
}

// ---------------------------------------------------------------------------
// System registration bridge
// ---------------------------------------------------------------------------

/// Wraps a C callback + userdata with a proper `Drop` for the userdata.
///
/// The raw `*mut c_void` isn't `Send + Sync`, but the plugin guarantees
/// thread-safe userdata (the engine is single-threaded anyway). We impl
/// Send + Sync manually so the closure can be stored in a `SystemFn`.
struct CallbackData {
    callback: SystemCallback,
    userdata: *mut c_void,
    drop_fn: Option<UserdataDrop>,
}

// SAFETY: The engine is single-threaded. Userdata crosses threads only in
// name (Send/Sync bounds on SystemFn); actual access is sequential.
// The plugin is responsible for ensuring userdata is safe to access.
unsafe impl Send for CallbackData {}
unsafe impl Sync for CallbackData {}

impl CallbackData {
    fn new(
        callback: SystemCallback,
        userdata: *mut c_void,
        drop_fn: Option<UserdataDrop>,
    ) -> Self {
        Self {
            callback,
            userdata,
            drop_fn,
        }
    }

    /// Invokes the callback with the given API.
    ///
    /// Using a method forces Rust 2021+ closures to capture the whole struct
    /// (not individual fields), so our `Send + Sync` impls apply.
    fn invoke(&self, api: &mut EngineApi) {
        (self.callback)(api as *mut EngineApi, self.userdata);
    }
}

impl Drop for CallbackData {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.drop_fn {
            unsafe { drop_fn(self.userdata) };
        }
    }
}

extern "C" fn bridge_add_system(
    ctx: *mut c_void,
    stage_u8: u8,
    callback: SystemCallback,
    userdata: *mut c_void,
    drop_fn: Option<UserdataDrop>,
) {
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };

    if bridge.schedule.is_null() {
        tracing::error!("add_system called outside of build() — schedule not available");
        return;
    }

    let schedule = unsafe { &mut *bridge.schedule };

    let stage = match stage_from_u8(stage_u8) {
        Some(s) => s,
        None => {
            tracing::error!(stage = stage_u8, "add_system called with invalid stage");
            return;
        }
    };

    let cb_data = CallbackData::new(callback, userdata, drop_fn);

    let system = move |resources: &mut Resources| {
        let mut runtime_ctx = BridgeContext {
            resources: resources as *mut Resources,
            schedule: std::ptr::null_mut(),
        };
        let mut api = create_engine_api(&mut runtime_ctx);
        cb_data.invoke(&mut api);
    };

    schedule.add_system(stage, system);
}

// ---------------------------------------------------------------------------
// Logging bridge
// ---------------------------------------------------------------------------

extern "C" fn bridge_log_info(
    _ctx: *mut c_void,
    msg_ptr: *const u8,
    msg_len: usize,
) {
    let msg = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(msg_ptr, msg_len)) };
    tracing::info!(target: "dynamic_plugin", "{msg}");
}

// ---------------------------------------------------------------------------
// Plugin data bridge
// ---------------------------------------------------------------------------

extern "C" fn bridge_set_plugin_data(
    ctx: *mut c_void,
    key_ptr: *const u8,
    key_len: usize,
    data_ptr: *const u8,
    data_len: usize,
) {
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };
    let resources = unsafe { &mut *bridge.resources };
    let key = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(key_ptr, key_len)) };
    let data = unsafe { std::slice::from_raw_parts(data_ptr, data_len) };

    if let Some(pd) = resources.get_mut::<PluginData>() {
        pd.set(key, data);
    } else {
        tracing::warn!("set_plugin_data called but no PluginData resource");
    }
}

extern "C" fn bridge_get_plugin_data(
    ctx: *mut c_void,
    key_ptr: *const u8,
    key_len: usize,
    out_len: *mut usize,
) -> *const u8 {
    let bridge = unsafe { &*(ctx as *const BridgeContext) };
    let resources = unsafe { &*bridge.resources };
    let key = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(key_ptr, key_len)) };

    let pd = match resources.get::<PluginData>() {
        Some(pd) => pd,
        None => {
            unsafe { *out_len = 0 };
            return std::ptr::null();
        }
    };

    match pd.get(key) {
        Some(data) => {
            unsafe { *out_len = data.len() };
            data.as_ptr()
        }
        None => {
            unsafe { *out_len = 0 };
            std::ptr::null()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn stage_from_u8(value: u8) -> Option<Stage> {
    match value {
        0 => Some(Stage::Startup),
        1 => Some(Stage::First),
        2 => Some(Stage::Input),
        3 => Some(Stage::PreUpdate),
        4 => Some(Stage::Update),
        5 => Some(Stage::PostUpdate),
        6 => Some(Stage::GpuSync),
        7 => Some(Stage::Gpu),
        8 => Some(Stage::Physics),
        9 => Some(Stage::PostPhysics),
        10 => Some(Stage::PreRender),
        11 => Some(Stage::Render),
        12 => Some(Stage::PostRender),
        13 => Some(Stage::Last),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_from_u8_valid() {
        assert_eq!(stage_from_u8(0), Some(Stage::Startup));
        assert_eq!(stage_from_u8(4), Some(Stage::Update));
        assert_eq!(stage_from_u8(13), Some(Stage::Last));
    }

    #[test]
    fn stage_from_u8_invalid() {
        assert_eq!(stage_from_u8(14), None);
        assert_eq!(stage_from_u8(255), None);
    }

    #[test]
    fn bridge_log_does_not_panic() {
        let msg = "test log message";
        bridge_log_info(
            std::ptr::null_mut(),
            msg.as_ptr(),
            msg.len(),
        );
    }

    #[test]
    fn bridge_spawn_without_entity_bridge() {
        let mut resources = Resources::new();
        let mut ctx = BridgeContext {
            resources: &mut resources as *mut Resources,
            schedule: std::ptr::null_mut(),
        };

        let result = bridge_spawn_entity(&mut ctx as *mut BridgeContext as *mut c_void);
        assert_eq!(result, 0);
    }

    #[test]
    fn bridge_plugin_data_roundtrip() {
        let mut resources = Resources::new();
        resources.insert(PluginData::new());

        let mut ctx = BridgeContext {
            resources: &mut resources as *mut Resources,
            schedule: std::ptr::null_mut(),
        };
        let ctx_ptr = &mut ctx as *mut BridgeContext as *mut c_void;

        let key = "test_key";
        let data = b"hello";
        bridge_set_plugin_data(
            ctx_ptr,
            key.as_ptr(), key.len(),
            data.as_ptr(), data.len(),
        );

        let mut out_len: usize = 0;
        let ptr = bridge_get_plugin_data(
            ctx_ptr,
            key.as_ptr(), key.len(),
            &mut out_len,
        );

        assert!(!ptr.is_null());
        assert_eq!(out_len, 5);
        let result = unsafe { std::slice::from_raw_parts(ptr, out_len) };
        assert_eq!(result, b"hello");
    }

    #[test]
    fn bridge_resource_access() {
        let mut resources = Resources::new();
        resources.insert(42_i32);

        let mut registry = ResourceRegistry::new();
        registry.register::<i32>("test::i32");
        resources.insert(registry);

        let mut ctx = BridgeContext {
            resources: &mut resources as *mut Resources,
            schedule: std::ptr::null_mut(),
        };
        let ctx_ptr = &mut ctx as *mut BridgeContext as *mut c_void;

        let name = "test::i32";
        let ptr = bridge_get_resource_ptr(ctx_ptr, name.as_ptr(), name.len());
        assert!(!ptr.is_null());

        let value = unsafe { &*(ptr as *const i32) };
        assert_eq!(*value, 42);

        // Unknown resource returns null.
        let name = "unknown";
        let ptr = bridge_get_resource_ptr(ctx_ptr, name.as_ptr(), name.len());
        assert!(ptr.is_null());
    }
}
