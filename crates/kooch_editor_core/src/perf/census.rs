//! A once-a-second line naming every long-lived container's size, so a leak names itself (#1592).
//!
//! 🔴 Not a permanent cost: it is a `HashMap` lookup per container, once a second, at `info` so it
//! reaches the console without `RUST_LOG`. It comes out when #1592 closes.

use std::time::{Duration, Instant};

use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_render::meshlet::MeshletRenderStage;

/// How often the census speaks. A leak of a megabyte a second is obvious at this rate.
const EVERY: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub(crate) struct CensusClock(Instant);

impl Default for CensusClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}

/// Logs what every cache holds, with the process's own RSS on the same line so the one that climbs
/// with it needs no correlating by hand.
///
/// 🔴 `rss_mb` reads 0 unless something is asking for system metrics — the Performance panel's
/// System section, or the Game View's info card. Forcing it here would defeat
/// `a_hidden_section_is_never_polled`, which exists so the editor does not poll the OS for a
/// number nobody is reading.
pub(crate) fn record(resources: &mut Resources, stage: Option<&MeshletRenderStage>, rss_mb: u32) {
    let mut clock = resources.remove::<CensusClock>().unwrap_or_default();
    if clock.0.elapsed() < EVERY {
        resources.insert(clock);
        return;
    }
    clock.0 = Instant::now();
    resources.insert(clock);

    let count = |resources: &Resources| {
        (
            assets_len::<kooch_render::mesh::Mesh>(resources),
            assets_len::<kooch_render::meshlet::MeshletMesh>(resources),
            assets_len::<kooch_render::texture::Image>(resources),
            assets_len::<kooch_render::material::Material>(resources),
            assets_len::<kooch_ecs::scene::SceneDocument>(resources),
        )
    };
    let (meshes, meshlets, images, materials, scenes) = count(resources);
    let cached = resources
        .get::<AssetServer>()
        .map_or(0, AssetServer::cached_count);
    let textures = resources
        .get::<kooch_render::material::MaterialPipeline>()
        .map_or(0, |materials| materials.texture_pool().len());
    let entities = resources
        .get::<kooch_ecs::archetype_registry::ArchetypeRegistry>()
        .map_or(0, |registry| {
            registry
                .iter_matching(&[])
                .map(|arch| arch.entities().len())
                .sum()
        });
    let (registered, pool_bytes, dead_bytes) = stage.map_or((0, 0, 0), |stage| {
        let pipeline = stage.pipeline();
        (
            pipeline.registered_count(),
            pipeline.pool().byte_size(),
            pipeline.dead_bytes(),
        )
    });
    let undo = resources
        .get::<crate::undo::UndoStack>()
        .map_or(0, crate::undo::UndoStack::depth);

    tracing::info!(
        target: "kooch_editor::census",
        rss_mb,
        meshes,
        meshlets,
        images,
        materials,
        scenes,
        cached,
        textures,
        entities,
        registered,
        pool_bytes,
        dead_bytes,
        undo,
        "census",
    );
}

fn assets_len<T: kooch_core::assets::Asset>(resources: &Resources) -> usize {
    resources.get::<Assets<T>>().map_or(0, Assets::<T>::len)
}
