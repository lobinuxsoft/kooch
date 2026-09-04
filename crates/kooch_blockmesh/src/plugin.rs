//! Registering the block component and its sync system.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use crate::{Block, BuiltBlocks, sync_blocks};

/// Registers [`Block`] and keeps every block's mesh and collider in step
/// with its source.
pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<Block>();
            }
            resources.insert(BuiltBlocks::default());
        });
        // Before physics, so a block edited this frame is collided
        // against this frame rather than next. Ungated: a level is built
        // while stopped, which is exactly when it has to be visible.
        app.add_system(Stage::PreUpdate, sync_blocks);
    }

    fn name(&self) -> &str {
        "BlockPlugin"
    }
}
