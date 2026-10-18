//! Registering the block component and its sync system.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::schedule::Order;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;

use crate::{
    BentMeshes, Block, BlockShape, BuiltBlocks, SplineDeform, SplineSweep, sync_blocks,
    sync_deforms,
};

/// Registers [`Block`] and keeps every block's mesh and collider in step
/// with its source.
pub struct BlockPlugin;

impl Plugin for BlockPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<Block>();
                registry.register_cpu_reflected::<BlockShape>();
                registry.register_cpu_reflected::<SplineSweep>();
                registry.register_cpu_reflected::<SplineDeform>();
            }
            resources.insert(BuiltBlocks::default());
            resources.insert(BentMeshes::default());
        });
        // 🔴 A block's collider is geometry the solver reads, so it is built in `PrePhysics` and
        // said out loud that it comes first (#1316). This used to sit in `PreUpdate` beside
        // `physics_sync_system`, where the only thing deciding which ran first was which plugin
        // had been added first — and a mesh that lost that race appeared a frame late without
        // failing. Ungated: a level is built while stopped, which is exactly when it has to be
        // visible.
        app.add_ordered(
            Stage::PrePhysics,
            Order::before("physics_sync_system"),
            sync_blocks,
        );
        // Beside `sync_blocks` and for the same reason: a bent mesh is geometry a level is built
        // from, so it exists while the scene is stopped, which is exactly when it is looked at.
        app.add_ordered(
            Stage::PrePhysics,
            Order::before("physics_sync_system"),
            sync_deforms,
        );
    }

    fn name(&self) -> &str {
        "BlockPlugin"
    }
}

#[cfg(test)]
mod tests;
