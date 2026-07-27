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

mod component_registration {
    use std::ffi::c_void;
    use std::sync::{Arc, Mutex};

    use ome_plugin_api::component::{ComponentDesc, FieldDesc, field_kind, register_result};

    use crate::dynamic::bridge::component::{
        ComponentBridge, PluginField, bridge_register_component,
    };
    use crate::dynamic::bridge::context::BridgeContext;
    use crate::resource::Resources;

    /// Records what the ECS side was asked to register, so the tests can
    /// assert on the decoded values rather than on a return code alone.
    type Seen = Arc<Mutex<Vec<(String, Vec<PluginField>)>>>;

    fn bridge_recording_into(seen: &Seen, accept: bool) -> ComponentBridge {
        let seen = Arc::clone(seen);
        ComponentBridge::new(move |_resources, name, fields| {
            seen.lock()
                .unwrap()
                .push((name.to_owned(), fields.to_vec()));
            accept
        })
    }

    fn call(resources: &mut Resources, desc: &ComponentDesc) -> u32 {
        let mut ctx = BridgeContext {
            resources: resources as *mut Resources,
            schedule: std::ptr::null_mut(),
        };
        bridge_register_component(
            &mut ctx as *mut BridgeContext as *mut c_void,
            desc as *const ComponentDesc,
        )
    }

    #[test]
    fn a_described_component_reaches_the_ecs_intact() {
        const FIELDS: &[FieldDesc] = &[
            FieldDesc::new("current", field_kind::U32),
            FieldDesc::new("regen", field_kind::F32),
        ];
        let seen: Seen = Arc::default();
        let mut resources = Resources::new();
        resources.insert(bridge_recording_into(&seen, true));

        let result = call(
            &mut resources,
            &ComponentDesc::new("my_game::Health", FIELDS),
        );

        assert_eq!(result, register_result::OK);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "my_game::Health");
        assert_eq!(
            seen[0].1,
            vec![
                PluginField {
                    name: "current".into(),
                    kind: field_kind::U32
                },
                PluginField {
                    name: "regen".into(),
                    kind: field_kind::F32
                },
            ]
        );
    }

    /// A marker component is a real thing, not a malformed description.
    #[test]
    fn zero_fields_is_accepted() {
        let seen: Seen = Arc::default();
        let mut resources = Resources::new();
        resources.insert(bridge_recording_into(&seen, true));

        let result = call(&mut resources, &ComponentDesc::new("my_game::Player", &[]));

        assert_eq!(result, register_result::OK);
        assert!(seen.lock().unwrap()[0].1.is_empty());
    }

    /// A plugin built against a newer API must fail on the field it
    /// cannot express, not register a type the editor would draw wrongly.
    #[test]
    fn an_unknown_field_kind_is_refused_rather_than_guessed() {
        const FIELDS: &[FieldDesc] = &[FieldDesc::new("mystery", field_kind::MAX + 1)];
        let seen: Seen = Arc::default();
        let mut resources = Resources::new();
        resources.insert(bridge_recording_into(&seen, true));

        let result = call(
            &mut resources,
            &ComponentDesc::new("my_game::Future", FIELDS),
        );

        assert_eq!(result, register_result::UNKNOWN_FIELD_KIND);
        assert!(
            seen.lock().unwrap().is_empty(),
            "nothing may reach the ECS once a field failed to decode"
        );
    }

    #[test]
    fn an_empty_name_is_refused() {
        let seen: Seen = Arc::default();
        let mut resources = Resources::new();
        resources.insert(bridge_recording_into(&seen, true));

        let result = call(&mut resources, &ComponentDesc::new("", &[]));

        assert_eq!(result, register_result::BAD_UTF8);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_name_the_ecs_rejects_reports_name_taken() {
        let seen: Seen = Arc::default();
        let mut resources = Resources::new();
        resources.insert(bridge_recording_into(&seen, false));

        let result = call(&mut resources, &ComponentDesc::new("my_game::Health", &[]));

        assert_eq!(result, register_result::NAME_TAKEN);
    }

    /// A host that built an `App` without the ECS gets a distinguishable
    /// failure rather than a silent no-op.
    #[test]
    fn without_a_bridge_it_says_so() {
        let mut resources = Resources::new();
        let result = call(&mut resources, &ComponentDesc::new("my_game::Health", &[]));
        assert_eq!(result, register_result::NO_BRIDGE);
    }

    /// The bridge is reinserted after use, so a second plugin can
    /// register too — the remove-use-reinsert dance must put it back.
    #[test]
    fn the_bridge_survives_a_registration() {
        let seen: Seen = Arc::default();
        let mut resources = Resources::new();
        resources.insert(bridge_recording_into(&seen, true));

        call(&mut resources, &ComponentDesc::new("my_game::A", &[]));
        let second = call(&mut resources, &ComponentDesc::new("my_game::B", &[]));

        assert_eq!(second, register_result::OK);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_null_description_does_not_dereference() {
        let mut resources = Resources::new();
        let mut ctx = BridgeContext {
            resources: &mut resources as *mut Resources,
            schedule: std::ptr::null_mut(),
        };
        let result = bridge_register_component(
            &mut ctx as *mut BridgeContext as *mut c_void,
            std::ptr::null(),
        );
        assert_eq!(result, register_result::BAD_UTF8);
    }
}

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
