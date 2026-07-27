use std::ffi::c_void;

use crate::dynamic::plugin_data::PluginData;
use crate::dynamic::resource_registry::ResourceRegistry;

use super::context::BridgeContext;

// ---------------------------------------------------------------------------
// Resource access bridge
// ---------------------------------------------------------------------------

pub(super) extern "C" fn bridge_get_resource_ptr(
    ctx: *mut c_void,
    name_ptr: *const u8,
    name_len: usize,
) -> *const c_void {
    let bridge = unsafe { &*(ctx as *const BridgeContext) };
    let resources = unsafe { &*bridge.resources };
    let name =
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len)) };

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

pub(super) extern "C" fn bridge_get_resource_mut_ptr(
    ctx: *mut c_void,
    name_ptr: *const u8,
    name_len: usize,
) -> *mut c_void {
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };
    let resources = unsafe { &mut *bridge.resources };
    let name =
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len)) };

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
// Logging bridge
// ---------------------------------------------------------------------------

pub(super) extern "C" fn bridge_log_info(_ctx: *mut c_void, msg_ptr: *const u8, msg_len: usize) {
    let msg =
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(msg_ptr, msg_len)) };
    tracing::info!(target: "dynamic_plugin", "{msg}");
}

// ---------------------------------------------------------------------------
// Plugin data bridge
// ---------------------------------------------------------------------------

pub(super) extern "C" fn bridge_set_plugin_data(
    ctx: *mut c_void,
    key_ptr: *const u8,
    key_len: usize,
    data_ptr: *const u8,
    data_len: usize,
) {
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };
    let resources = unsafe { &mut *bridge.resources };
    let key =
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(key_ptr, key_len)) };
    let data = unsafe { std::slice::from_raw_parts(data_ptr, data_len) };

    if let Some(pd) = resources.get_mut::<PluginData>() {
        pd.set(key, data);
    } else {
        tracing::warn!("set_plugin_data called but no PluginData resource");
    }
}

pub(super) extern "C" fn bridge_get_plugin_data(
    ctx: *mut c_void,
    key_ptr: *const u8,
    key_len: usize,
    out_len: *mut usize,
) -> *const u8 {
    let bridge = unsafe { &*(ctx as *const BridgeContext) };
    let resources = unsafe { &*bridge.resources };
    let key =
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(key_ptr, key_len)) };

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
