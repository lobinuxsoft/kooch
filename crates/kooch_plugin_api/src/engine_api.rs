//! What a plugin can ask the engine to do: [`Engine`] is a plain trait object, sound because both
//! sides share a compiler, which [`version`](crate::version) checks first.

use crate::component::{ComponentSchema, RegisterError};
use crate::types::{Order, Stage};

/// A plugin system, run each frame with the same [`Engine`] handle the plugin got at build time.
pub type PluginSystem = Box<dyn FnMut(&mut dyn Engine) + Send + Sync>;

/// The engine services available to a plugin, passed to [`KoochPlugin`](crate::KoochPlugin) as
/// `&mut dyn Engine`.
pub trait Engine {
    /// Spawns an entity and returns its packed handle (see
    /// [`pack_entity`](crate::types::pack_entity)); `None` without an entity allocator.
    fn spawn_entity(&mut self) -> Option<u64>;

    /// Despawns an entity. `false` if the handle was already stale.
    fn despawn_entity(&mut self, entity: u64) -> bool;

    /// Declares a component type this plugin owns, stored by the schema's `type_name` — hence
    /// stable across rebuilds.
    fn register_component(&mut self, schema: ComponentSchema) -> Result<(), RegisterError>;

    /// Registers a system to run at `stage` every frame.
    fn add_system(&mut self, stage: Stage, system: PluginSystem);

    /// Registers a system that runs where `order` puts it inside `stage` — how a plugin runs after
    /// an engine pass without knowing which plugin loaded first (#392).
    fn add_ordered(&mut self, stage: Stage, order: Order, system: PluginSystem) {
        let _ = order;
        self.add_system(stage, system);
    }

    /// Registers a render pass, erased so this crate names no GPU type. `pass` is a
    /// `Box<Box<dyn kooch_plugin_render::RenderPass>>`; use `RenderEngine::add_pass` from
    /// `kooch_plugin_render` rather than calling this (#392).
    ///
    /// `false` when the host refused it: outside `build()`, or without a GPU.
    fn add_pass_erased(
        &mut self,
        stage: Stage,
        order: Order,
        pass: Box<dyn std::any::Any + Send + Sync>,
    ) -> bool {
        let _ = (stage, order, pass);
        false
    }

    /// Writes a line to the engine's log.
    fn log(&self, message: &str);

    /// Stores bytes under `key`, owned by the host — how plugin state survives a reload that
    /// discards the library's statics.
    fn set_data(&mut self, key: &str, data: &[u8]);

    /// Reads back what [`set_data`](Engine::set_data) stored.
    fn get_data(&self, key: &str) -> Option<&[u8]>;
}
