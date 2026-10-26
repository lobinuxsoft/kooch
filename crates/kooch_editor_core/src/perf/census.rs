//! A line naming every long-lived container's size whenever one changes, so what a scene change
//! keeps names itself (#1589, first used for #1592).
//!
//! 🔴 Temporary: checked once a second, logged at `info` only on a change. Comes out with the fix.

use std::time::{Duration, Instant};

use kooch_core::asset_loader::AssetServer;
use kooch_core::assets::Assets;
use kooch_core::resource::Resources;
use kooch_render::meshlet::MeshletRenderStage;

/// How often the census speaks. A leak of a megabyte a second is obvious at this rate.
const EVERY: Duration = Duration::from_secs(1);

/// When the census last looked, and what it said then.
#[derive(Debug)]
pub(crate) struct CensusClock(Instant, Vec<usize>, u64);

impl Default for CensusClock {
    fn default() -> Self {
        Self(Instant::now(), Vec::new(), 0)
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

    let count = |resources: &Resources| {
        (
            assets_len::<kooch_render::mesh::Mesh>(resources),
            assets_len::<kooch_render::meshlet::MeshletMesh>(resources),
            assets_len::<kooch_render::texture::Image>(resources),
            assets_len::<kooch_render::material::Material>(resources),
            assets_len::<kooch_ecs::scene::SceneDocument>(resources),
            assets_len::<kooch_render::material::Shader>(resources),
        )
    };
    let (meshes, meshlets, images, materials, scenes, shaders) = count(resources);
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

    let memory = Memory::sample();
    let shape = vec![
        meshes, meshlets, images, materials, scenes, shaders, cached, textures, entities, undo,
    ];
    let quiet = shape == clock.1 && memory.heap_used.abs_diff(clock.2) < 2;
    clock.1 = shape;
    clock.2 = memory.heap_used;
    resources.insert(clock);
    if quiet {
        return;
    }

    tracing::info!(
        target: "kooch_editor::census",
        rss_mb,
        heap_used = memory.heap_used,
        heap_free = memory.heap_free,
        heap_mmap = memory.heap_mmap,
        rss_anon = memory.rss_anon,
        rss_file = memory.rss_file,
        rss_shmem = memory.rss_shmem,
        fds = memory.fds,
        meshes,
        meshlets,
        images,
        materials,
        scenes,
        shaders,
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

/// Where the process's memory is, in MB, so one capture separates the candidates (#1592): a climbing
/// `heap_used` or `heap_mmap` is a real leak, `heap_free` alone is fragmentation, `rss_file` /
/// `rss_shmem` are mappings, `rss_anon` with both heap figures flat is memory that never went
/// through malloc, `fds` is descriptors.
///
/// 🔴 glibc serves large allocations with `mmap`, so a leaked big buffer climbs `heap_mmap` and
/// leaves `heap_used` still: a 64 MiB `Vec` measured 0 → 64 there and 0 → 0 in the arena.
#[derive(Default)]
struct Memory {
    heap_used: u64,
    heap_free: u64,
    heap_mmap: u64,
    rss_anon: u64,
    rss_file: u64,
    rss_shmem: u64,
    fds: usize,
}

impl Memory {
    fn sample() -> Self {
        let mut memory = Self::default();
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        {
            // SAFETY: `mallinfo2` takes no arguments, returns by value and only reads glibc's
            // arena counters.
            let info = unsafe { libc::mallinfo2() };
            memory.heap_used = info.uordblks as u64 / MB;
            memory.heap_free = info.fordblks as u64 / MB;
            memory.heap_mmap = info.hblkhd as u64 / MB;
        }
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                let kb = |line: &str| {
                    line.split_whitespace()
                        .nth(1)
                        .and_then(|n| n.parse::<u64>().ok())
                        .map_or(0, |kb| kb / 1024)
                };
                match line.split(':').next() {
                    Some("RssAnon") => memory.rss_anon = kb(line),
                    Some("RssFile") => memory.rss_file = kb(line),
                    Some("RssShmem") => memory.rss_shmem = kb(line),
                    _ => {}
                }
            }
        }
        memory.fds = std::fs::read_dir("/proc/self/fd").map_or(0, Iterator::count);
        memory
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
const MB: u64 = 1024 * 1024;
