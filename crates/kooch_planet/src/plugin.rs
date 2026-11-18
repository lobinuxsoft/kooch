//! Registering [`Planet`] and the system that draws every planet.

use std::any::TypeId;

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;
use kooch_ecs::ephemeral::EphemeralComponents;

use crate::component::{Planet, PlanetBlock};
use crate::sync::{PlanetStats, PlanetTerrain, sync_planets};

/// Registers [`Planet`] and keeps every planet's blocks generated and drawn for the cameras.
///
/// Add it wherever a scene with planets loads; it draws only where there is a GPU, so a headless
/// host keeps the component and spends nothing on it.
pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<Planet>();
            }
            match resources.get_mut::<EphemeralComponents>() {
                Some(ephemeral) => ephemeral.insert(TypeId::of::<PlanetBlock>()),
                None => tracing::warn!(
                    target: "kooch_planet",
                    "no EphemeralComponents: planet blocks would be saved into the scene",
                ),
            }
            resources.insert(PlanetTerrain::default());
            resources.insert(PlanetStats::default());
        });
        // After `PostUpdate`'s transform propagation, so a planet's blocks follow where it is
        // this frame, and before `PreRender` gathers what to draw.
        app.add_system(Stage::Gpu, sync_planets);
    }

    fn name(&self) -> &str {
        "PlanetPlugin"
    }
}
