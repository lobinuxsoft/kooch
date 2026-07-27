//! The trait a plugin implements, and the symbols it exports.
//!
//! # Example
//!
//! ```ignore
//! use ome_plugin_api::prelude::*;
//!
//! #[derive(Default)]
//! struct MyPlugin;
//!
//! impl OmePlugin for MyPlugin {
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
//! ome_plugin_api::export_plugin!(MyPlugin);
//! ```

use crate::engine_api::Engine;

/// A dynamically loaded plugin.
///
/// # Reloading
///
/// **A plugin owns no state that has to survive a reload.** The library
/// is unloaded and replaced, and anything living in its statics goes
/// with it; state that must persist belongs to the host, through
/// [`Engine::set_data`]. Every project that has built this arrives at
/// the same rule, and it is not one the type system can enforce.
pub trait OmePlugin: Send + Sync {
    /// Name for logs and diagnostics.
    fn name(&self) -> &str;

    /// Registers the plugin's components and systems.
    ///
    /// Called once after loading, and again after every reload.
    fn build(&mut self, engine: &mut dyn Engine);

    /// Releases what the plugin holds, before it is unloaded.
    fn cleanup(&mut self) {}
}

/// Signature of the constructor a plugin exports as `ome_create_plugin`.
///
/// `Box<dyn OmePlugin>` is not FFI-safe in the general case, which is
/// precisely why [`version::check`](crate::version::check) runs first:
/// it refuses to load anything not built by the same compiler against
/// the same API version, and that agreement is the condition under
/// which handing a Rust trait object across the boundary is sound.
// The compiler is right that a fat pointer is not FFI-safe in general,
// and that warning is exactly what the build stamp answers: the symbol
// is never called until the loader has proven both sides came from the
// same compiler and the same API version. Silenced here, once, with the
// reason attached — rather than left to fire at every call site.
#[allow(improper_ctypes_definitions)]
pub type CreatePluginFn = unsafe extern "C" fn() -> Box<dyn OmePlugin>;

/// Symbol name of the constructor, for `libloading`.
pub const CREATE_SYMBOL: &[u8] = b"ome_create_plugin";

/// Symbol name of the build stamp the loader verifies first.
pub const STAMP_SYMBOL: &[u8] = b"ome_plugin_build_stamp";

/// Exports a plugin type as a loadable library.
///
/// Emits both symbols the loader needs: the build stamp it checks before
/// anything else, and the constructor it calls once the stamp matched.
/// The type must implement [`Default`].
#[macro_export]
macro_rules! export_plugin {
    ($ty:ty) => {
        /// Identifies the API and compiler this plugin was built with.
        /// The loader reads this before calling anything else.
        #[unsafe(no_mangle)]
        pub extern "C" fn ome_plugin_build_stamp() -> $crate::version::BuildStamp {
            $crate::version::BuildStamp::current()
        }

        /// Constructs the plugin. Only sound once the stamp matched,
        /// which is why the loader reads the stamp first.
        #[unsafe(no_mangle)]
        #[allow(improper_ctypes_definitions)]
        pub extern "C" fn ome_create_plugin() -> ::std::boxed::Box<dyn $crate::OmePlugin> {
            ::std::boxed::Box::new(<$ty as ::std::default::Default>::default())
        }
    };
}
