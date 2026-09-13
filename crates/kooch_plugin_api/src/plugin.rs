//! The trait a plugin implements, and the symbols it exports.
//!
//! # Example
//!
//! ```ignore
//! use kooch_plugin_api::prelude::*;
//!
//! #[derive(Default)]
//! struct MyPlugin;
//!
//! impl KoochPlugin for MyPlugin {
//!     fn name(&self) -> &str {
//!         "MyPlugin"
//!     }
//!
//!     fn build(&mut self, engine: &mut dyn Engine) {
//!         engine
//!             .register_component(
//!                 ComponentSchema::new("my_game::Health")
//!                     .with_field("current", FieldKind::U32)
//!                     .with_field("max", FieldKind::U32),
//!             )
//!             .expect("Health");
//!
//!         engine.add_system(Stage::Update, Box::new(|engine| engine.log("tick")));
//!     }
//! }
//!
//! kooch_plugin_api::export_plugin!(MyPlugin);
//! ```

use crate::engine_api::Engine;

/// A dynamically loaded plugin. **It owns no state that must survive a reload**; persistent state
/// belongs to the host via [`Engine::set_data`].
pub trait KoochPlugin: Send + Sync {
    /// Name for logs and diagnostics.
    fn name(&self) -> &str;

    /// Registers the plugin's components and systems.
    ///
    /// Called once after loading, and again after every reload.
    fn build(&mut self, engine: &mut dyn Engine);

    /// Releases what the plugin holds, before it is unloaded.
    fn cleanup(&mut self) {}
}

/// Signature of the `kooch_create_plugin` constructor. Not FFI-safe in general, which is why the
/// [`BuildStamp`](crate::version::BuildStamp) check proves one compiler and API first.
// Silenced once: the build stamp is what makes this fat pointer sound.
#[allow(improper_ctypes_definitions)]
pub type CreatePluginFn = unsafe extern "C" fn() -> Box<dyn KoochPlugin>;

/// Symbol name of the constructor, for `libloading`.
pub const CREATE_SYMBOL: &[u8] = b"kooch_create_plugin";

/// Symbol name of the build stamp the loader verifies first.
pub const STAMP_SYMBOL: &[u8] = b"kooch_plugin_build_stamp";

/// Exports a plugin type as a loadable library: the build stamp the loader checks first, and the
/// constructor it calls after. The type must implement [`Default`].
#[macro_export]
macro_rules! export_plugin {
    ($ty:ty) => {
        /// Identifies the API and compiler this plugin was built with.
        /// The loader reads this before calling anything else.
        #[unsafe(no_mangle)]
        pub extern "C" fn kooch_plugin_build_stamp() -> $crate::version::BuildStamp {
            $crate::version::BuildStamp::current()
        }

        /// Constructs the plugin. Only sound once the stamp matched,
        /// which is why the loader reads the stamp first.
        #[unsafe(no_mangle)]
        #[allow(improper_ctypes_definitions)]
        pub extern "C" fn kooch_create_plugin() -> ::std::boxed::Box<dyn $crate::KoochPlugin> {
            ::std::boxed::Box::new(<$ty as ::std::default::Default>::default())
        }
    };
}
