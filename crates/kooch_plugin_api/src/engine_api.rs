//! What a plugin can ask the engine to do.
//!
//! [`Engine`] is an ordinary Rust trait. The plugin is a `dylib` built
//! by the same compiler as the host, so a trait object crosses the
//! boundary as itself — no function-pointer table, no `*mut c_void`, no
//! stable-ABI layer.
//!
//! That compiler agreement is the whole contract, and it is checked
//! before any of this is called: see [`version`](crate::version).

use crate::component::{ComponentSchema, RegisterError};
use crate::types::Stage;

/// A system a plugin registers, run by the engine each frame.
///
/// It receives the same [`Engine`] handle the plugin got at build time,
/// so a system can spawn, log and read plugin data without capturing
/// anything the engine cannot see.
pub type PluginSystem = Box<dyn FnMut(&mut dyn Engine) + Send + Sync>;

/// The engine services available to a plugin.
///
/// Implemented by the host, passed to [`KoochPlugin`](crate::KoochPlugin) as
/// `&mut dyn Engine`. A plugin never constructs one.
pub trait Engine {
    /// Spawns an entity and returns its packed handle.
    ///
    /// See [`pack_entity`](crate::types::pack_entity) for the layout.
    /// Returns `None` if the host has no entity allocator.
    fn spawn_entity(&mut self) -> Option<u64>;

    /// Despawns an entity. `false` if the handle was already stale.
    fn despawn_entity(&mut self, entity: u64) -> bool;

    /// Declares a component type this plugin owns.
    ///
    /// The engine cannot name the plugin's Rust types, so it stores them
    /// by the schema's `type_name` — which is why that name has to stay
    /// stable across rebuilds.
    fn register_component(&mut self, schema: ComponentSchema) -> Result<(), RegisterError>;

    /// Registers a system to run at `stage` every frame.
    fn add_system(&mut self, stage: Stage, system: PluginSystem);

    /// Writes a line to the engine's log.
    fn log(&self, message: &str);

    /// Stores bytes under `key`, owned by the host.
    ///
    /// This is how a plugin keeps state across a reload: the library is
    /// unloaded and rebuilt, but anything parked here belongs to the
    /// host and survives. State held in the plugin's own statics does
    /// not — it goes away with the library.
    fn set_data(&mut self, key: &str, data: &[u8]);

    /// Reads back what [`set_data`](Engine::set_data) stored.
    fn get_data(&self, key: &str) -> Option<&[u8]>;
}
