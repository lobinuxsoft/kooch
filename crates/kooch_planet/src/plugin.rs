//! Registering [`Terrain`] and the system that draws every planet.

use kooch_core::app::App;
use kooch_core::plugin::Plugin;
use kooch_core::resource::Resources;
use kooch_core::stage::Stage;
use kooch_ecs::component::ComponentRegistry;
use kooch_render::meshlet::Submitted;

use crate::component::Terrain;
use crate::sync::{PlanetStats, PlanetTerrain, sync_planets};

/// Registers [`Terrain`] and keeps every planet's blocks generated and drawn for the cameras.
///
/// Add it wherever a scene with planets loads; it draws only where there is a GPU, so a headless
/// host keeps the component and spends nothing on it.
pub struct PlanetPlugin;

impl Plugin for PlanetPlugin {
    fn build(&self, app: &mut App) {
        app.add_system(Stage::Startup, |resources: &mut Resources| {
            if let Some(registry) = resources.get_mut::<ComponentRegistry>() {
                registry.register_cpu_reflected::<Terrain>();
            }
            // Where the blocks are drawn from: they are no entities (#1991).
            if !resources.contains::<Submitted>() {
                resources.insert(Submitted::default());
            }
            resources.insert(PlanetTerrain::default());
            resources.insert(PlanetStats::default());
            #[cfg(feature = "physics")]
            crate::ground::register(resources);
        });
        // After `PostUpdate`'s transform propagation, so a planet's blocks follow where it is
        // this frame, and before `PreRender` gathers what to draw.
        app.add_system(Stage::Gpu, sync_planets);
        // `Update` runs before `PrePhysics` in a frame, so a block added here collides this step;
        // the generators it reads are the ones `sync_planets` loaded last frame. Only while playing:
        // nothing simulates in edit mode, so stopping takes every block back out.
        #[cfg(feature = "physics")]
        {
            app.add_system(
                Stage::Update,
                kooch_core::run_state::run_if_playing(crate::ground::sync_ground),
            );
            app.add_system(Stage::Update, crate::ground::release_ground);
        }
    }

    fn name(&self) -> &str {
        "PlanetPlugin"
    }
}
