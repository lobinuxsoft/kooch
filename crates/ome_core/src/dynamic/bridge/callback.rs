// ---------------------------------------------------------------------------
// System registration bridge
// ---------------------------------------------------------------------------

use std::ffi::c_void;

use ome_plugin_api::engine_api::EngineApi;
use ome_plugin_api::types::{SystemCallback, UserdataDrop};

use crate::resource::Resources;
use crate::stage::Stage;

use super::context::BridgeContext;
use super::create_engine_api;

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

pub(super) extern "C" fn bridge_add_system(
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
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn stage_from_u8(value: u8) -> Option<Stage> {
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
