use std::ffi::c_void;

use crate::dynamic::plugin_data::PluginData;
use crate::dynamic::resource_registry::ResourceRegistry;
use crate::resource::Resources;
use crate::stage::Stage;

use super::callback::stage_from_u8;
use super::context::BridgeContext;
use super::entity::bridge_spawn_entity;
use super::resource_api::{
    bridge_get_plugin_data, bridge_get_resource_ptr, bridge_log_info, bridge_set_plugin_data,
};

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
    bridge_log_info(std::ptr::null_mut(), msg.as_ptr(), msg.len());
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
    bridge_set_plugin_data(ctx_ptr, key.as_ptr(), key.len(), data.as_ptr(), data.len());

    let mut out_len: usize = 0;
    let ptr = bridge_get_plugin_data(ctx_ptr, key.as_ptr(), key.len(), &mut out_len);

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
