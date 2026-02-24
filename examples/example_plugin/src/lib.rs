//! Example dynamic plugin for OhMyEngine.
//!
//! Demonstrates the minimal setup to create a loadable `.dll`/`.so` plugin:
//! - Implement [`OmePlugin`]
//! - Export a constructor via `#[stabby::export]`
//!
//! Build with: `cargo build -p example_plugin`
//! Load with: `examples/dynamic_plugin.rs`

use std::ffi::c_void;

use ome_plugin_api::prelude::*;
use ome_plugin_api::BoxedPlugin;

/// A minimal example plugin that logs messages and registers a system.
struct HelloPlugin;

impl OmePlugin for HelloPlugin {
    extern "C" fn name(&self) -> stabby::string::String {
        "HelloPlugin".into()
    }

    extern "C" fn api_version(&self) -> u32 {
        API_VERSION
    }

    extern "C" fn build(&mut self, api: *mut EngineApi) {
        let api = unsafe { &mut *api };
        api.log("HelloPlugin::build() — registering systems");

        // Store some plugin data for demonstration.
        api.set_data("hello.greeting", b"Hello from a dynamic plugin!");

        // Register a system that runs every frame in the Update stage.
        // No userdata needed — we use null.
        api.register_system(
            stage::UPDATE,
            hello_system,
            std::ptr::null_mut(),
            None,
        );

        // Register a system with userdata.
        let counter = Box::new(CounterState { count: 0 });
        let userdata = Box::into_raw(counter) as *mut c_void;
        api.register_system(
            stage::POST_UPDATE,
            counter_system,
            userdata,
            Some(drop_counter),
        );
    }

    extern "C" fn cleanup(&mut self) {
        // Nothing to clean up in this example.
    }
}

/// System callback that logs a message each frame.
extern "C" fn hello_system(api: *mut EngineApi, _userdata: *mut c_void) {
    let api = unsafe { &mut *api };

    // Read back the data we stored during build.
    if let Some(greeting) = api.get_data("hello.greeting") {
        let msg = std::str::from_utf8(greeting).unwrap_or("(invalid utf8)");
        api.log(msg);
    }
}

/// Per-system state stored as userdata.
struct CounterState {
    count: u32,
}

/// System that counts frames using userdata.
extern "C" fn counter_system(api: *mut EngineApi, userdata: *mut c_void) {
    let api = unsafe { &mut *api };
    let state = unsafe { &mut *(userdata as *mut CounterState) };

    state.count += 1;

    if state.count % 60 == 0 {
        let msg = format!("CounterSystem: {} frames elapsed", state.count);
        api.log(&msg);
    }
}

/// Destructor for CounterState userdata.
unsafe extern "C" fn drop_counter(userdata: *mut c_void) {
    drop(unsafe { Box::from_raw(userdata as *mut CounterState) });
}

/// Plugin constructor exported for the engine to find.
///
/// The engine calls `lib.get_stabbied(b"ome_create_plugin")` to load this.
#[stabby::export]
extern "C" fn ome_create_plugin() -> BoxedPlugin {
    stabby::alloc::boxed::Box::new(HelloPlugin).into()
}
