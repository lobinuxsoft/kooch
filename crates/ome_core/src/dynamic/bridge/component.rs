// ---------------------------------------------------------------------------
// Component bridge — delegates to ComponentBridge resource (registered by ome_ecs)
// ---------------------------------------------------------------------------

use std::ffi::c_void;

use ome_plugin_api::component::{ComponentDesc, field_kind, register_result};

use crate::resource::Resources;

use super::context::BridgeContext;

/// One field of a plugin-declared component, decoded from FFI.
///
/// Owned rather than borrowed: the plugin's pointers are only valid for
/// the duration of the registration call, and the registry outlives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginField {
    /// Field name as declared by the plugin.
    pub name: String,
    /// One of the `field_kind` constants, already validated as known.
    pub kind: u8,
}

/// Registers component types declared by dynamically loaded plugins.
///
/// Stored as a resource with a closure inside, so `ome_core` does not
/// depend on `ome_ecs` types — the same shape as
/// [`EntityBridge`](super::EntityBridge), for the same reason.
///
/// The closure returns `false` when a *different* type already holds the
/// name, which is the one failure the bridge cannot judge for itself.
pub struct ComponentBridge {
    register_fn: Box<dyn Fn(&mut Resources, &str, &[PluginField]) -> bool + Send + Sync>,
}

impl ComponentBridge {
    /// Creates a bridge from the registration logic the ECS provides.
    pub fn new(
        register: impl Fn(&mut Resources, &str, &[PluginField]) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self {
            register_fn: Box::new(register),
        }
    }
}

/// Reads a plugin-provided UTF-8 string, or `None` if it is not valid.
///
/// A null pointer reads as the empty string rather than dereferencing:
/// a plugin declaring no name is a caller error, and it is rejected by
/// the emptiness check rather than by a crash.
unsafe fn str_from_raw<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() {
        return (len == 0).then_some("");
    }
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(ptr, len) }).ok()
}

pub(super) extern "C" fn bridge_register_component(
    ctx: *mut c_void,
    desc: *const ComponentDesc,
) -> u32 {
    if ctx.is_null() || desc.is_null() {
        return register_result::BAD_UTF8;
    }
    let bridge = unsafe { &mut *(ctx as *mut BridgeContext) };
    let resources = unsafe { &mut *bridge.resources };
    let desc = unsafe { &*desc };

    let Some(name) = (unsafe { str_from_raw(desc.name_ptr, desc.name_len) }) else {
        tracing::warn!("plugin registered a component whose name is not valid UTF-8");
        return register_result::BAD_UTF8;
    };
    if name.is_empty() {
        tracing::warn!("plugin registered a component with an empty name");
        return register_result::BAD_UTF8;
    }

    // A marker component has no fields, so an empty slice is legal — but
    // `from_raw_parts` requires a non-null aligned pointer even for
    // length zero, so the null case is handled before it is called.
    let fields_raw: &[ome_plugin_api::component::FieldDesc] =
        if desc.field_count == 0 || desc.fields_ptr.is_null() {
            &[]
        } else {
            unsafe { std::slice::from_raw_parts(desc.fields_ptr, desc.field_count) }
        };

    let mut fields = Vec::with_capacity(fields_raw.len());
    for raw in fields_raw {
        let Some(field_name) = (unsafe { str_from_raw(raw.name_ptr, raw.name_len) }) else {
            tracing::warn!(component = name, "plugin field name is not valid UTF-8");
            return register_result::BAD_UTF8;
        };
        // Reject rather than guess: a plugin built against a newer API
        // must fail on the field it cannot express here, not register a
        // type the editor would then draw wrongly.
        if raw.kind > field_kind::MAX {
            tracing::warn!(
                component = name,
                field = field_name,
                kind = raw.kind,
                "unknown field kind — plugin built against a newer API?"
            );
            return register_result::UNKNOWN_FIELD_KIND;
        }
        fields.push(PluginField {
            name: field_name.to_owned(),
            kind: raw.kind,
        });
    }

    // Remove-use-reinsert to avoid aliasing, as the entity bridge does.
    let Some(component_bridge) = resources.remove::<ComponentBridge>() else {
        tracing::warn!(
            component = name,
            "register_component called but no ComponentBridge registered"
        );
        return register_result::NO_BRIDGE;
    };
    let accepted = (component_bridge.register_fn)(resources, name, &fields);
    resources.insert(component_bridge);

    if accepted {
        tracing::info!(
            component = name,
            fields = fields.len(),
            "plugin component registered"
        );
        register_result::OK
    } else {
        tracing::warn!(
            component = name,
            "component name already taken by another type"
        );
        register_result::NAME_TAKEN
    }
}
