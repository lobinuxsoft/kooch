//! Engine API exposed to plugins via function pointer table.
//!
//! [`EngineApi`] is a `#[repr(C)]` struct of function pointers giving plugins
//! access to engine functionality. Every pointer uses the C calling convention
//! and passes the opaque `ctx` as the first argument (classic C pattern).
//!
//! Plugin authors should use the safe convenience methods on `EngineApi` rather
//! than calling function pointers directly.

use std::ffi::c_void;

use crate::types::{SystemCallback, UserdataDrop};

/// Function pointer table giving plugins access to engine services.
///
/// Passed to [`OmePlugin::build`](crate::OmePlugin::build) during loading and
/// to [`SystemCallback`] during execution. All pointers use C calling convention.
///
/// # Safety
///
/// - `ctx` is an opaque pointer owned by the engine. Plugins must not free it
///   or use it outside of callbacks.
/// - String parameters (`*const u8`, `usize`) must be valid UTF-8.
/// - Resource pointers from `get_resource_ptr`/`get_resource_mut_ptr` are only
///   valid for the duration of the current callback invocation.
#[repr(C)]
pub struct EngineApi {
    /// Opaque engine context. Forwarded to every function pointer call.
    pub ctx: *mut c_void,

    /// Spawns a new entity. Returns a packed `u64` handle (see [`pack_entity`](crate::types::pack_entity)).
    /// Returns 0 if no entity system is registered.
    pub spawn_entity: extern "C" fn(ctx: *mut c_void) -> u64,

    /// Despawns an entity by packed handle. Returns 1 on success, 0 if stale.
    pub despawn_entity: extern "C" fn(ctx: *mut c_void, entity: u64) -> u32,

    /// Gets an immutable pointer to a resource by name.
    /// Returns null if the resource is not found.
    pub get_resource_ptr:
        extern "C" fn(ctx: *mut c_void, name_ptr: *const u8, name_len: usize) -> *const c_void,

    /// Gets a mutable pointer to a resource by name.
    /// Returns null if the resource is not found.
    pub get_resource_mut_ptr:
        extern "C" fn(ctx: *mut c_void, name_ptr: *const u8, name_len: usize) -> *mut c_void,

    /// Registers a system at the given stage.
    ///
    /// - `stage`: one of the [`stage`](crate::types::stage) constants
    /// - `callback`: function called each frame
    /// - `userdata`: opaque pointer forwarded to callback
    /// - `drop_fn`: optional destructor for userdata (null = no-op)
    pub add_system: extern "C" fn(
        ctx: *mut c_void,
        stage: u8,
        callback: SystemCallback,
        userdata: *mut c_void,
        drop_fn: Option<UserdataDrop>,
    ),

    /// Logs an info-level message.
    pub log_info: extern "C" fn(ctx: *mut c_void, msg_ptr: *const u8, msg_len: usize),

    /// Stores plugin-specific data by key (engine copies both key and value).
    pub set_plugin_data: extern "C" fn(
        ctx: *mut c_void,
        key_ptr: *const u8,
        key_len: usize,
        data_ptr: *const u8,
        data_len: usize,
    ),

    /// Retrieves plugin-specific data by key.
    ///
    /// On success returns a pointer to the data and writes the length to `out_len`.
    /// Returns null if the key is not found. The pointer is valid until the next
    /// `set_plugin_data` call with the same key.
    pub get_plugin_data: extern "C" fn(
        ctx: *mut c_void,
        key_ptr: *const u8,
        key_len: usize,
        out_len: *mut usize,
    ) -> *const u8,
}

// SAFETY: EngineApi is a bag of function pointers + an opaque context pointer.
// The engine guarantees it is only used on the main thread.
unsafe impl Send for EngineApi {}
unsafe impl Sync for EngineApi {}

/// Safe convenience methods for plugin authors.
///
/// These wrap the raw function pointer calls with idiomatic Rust signatures.
impl EngineApi {
    /// Spawns a new entity, returning its packed handle.
    ///
    /// Returns 0 if no entity system is registered.
    #[inline]
    pub fn spawn(&mut self) -> u64 {
        (self.spawn_entity)(self.ctx)
    }

    /// Despawns an entity by packed handle. Returns `true` if successful.
    #[inline]
    pub fn despawn(&mut self, entity: u64) -> bool {
        (self.despawn_entity)(self.ctx, entity) != 0
    }

    /// Gets an immutable pointer to a named resource.
    ///
    /// Returns null if the resource is not registered.
    #[inline]
    pub fn resource_ptr(&self, name: &str) -> *const c_void {
        (self.get_resource_ptr)(self.ctx, name.as_ptr(), name.len())
    }

    /// Gets a mutable pointer to a named resource.
    ///
    /// Returns null if the resource is not registered.
    #[inline]
    pub fn resource_mut_ptr(&mut self, name: &str) -> *mut c_void {
        (self.get_resource_mut_ptr)(self.ctx, name.as_ptr(), name.len())
    }

    /// Registers a system callback to run at the given stage.
    #[inline]
    pub fn register_system(
        &mut self,
        stage: u8,
        callback: SystemCallback,
        userdata: *mut c_void,
        drop_fn: Option<UserdataDrop>,
    ) {
        (self.add_system)(self.ctx, stage, callback, userdata, drop_fn)
    }

    /// Logs an info-level message.
    #[inline]
    pub fn log(&self, msg: &str) {
        (self.log_info)(self.ctx, msg.as_ptr(), msg.len())
    }

    /// Stores plugin data by key.
    #[inline]
    pub fn set_data(&mut self, key: &str, data: &[u8]) {
        (self.set_plugin_data)(self.ctx, key.as_ptr(), key.len(), data.as_ptr(), data.len())
    }

    /// Retrieves plugin data by key. Returns `None` if not found.
    ///
    /// The returned slice is valid until the next `set_data` call with the same key.
    pub fn get_data(&self, key: &str) -> Option<&[u8]> {
        let mut len: usize = 0;
        let ptr = (self.get_plugin_data)(self.ctx, key.as_ptr(), key.len(), &mut len);
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { std::slice::from_raw_parts(ptr, len) })
        }
    }
}
