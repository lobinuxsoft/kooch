//! Each frame: which blocks every planet holds for the cameras, the new ones sent to the GPU, and
//! the meshes that came back handed to [`draw`](crate::draw).
//!
//! 🔴 Runs where the frame is drawn, never where it is simulated: the editor's remote host has no
//! GPU, so the editor draws a playing game's planets itself, as it does a block's mesh.

use glam::{DVec3, Mat4};
use kooch_core::Guid;
use kooch_core::asset_loader::{AssetServer, ReloadedAssets};
use kooch_core::assets::{Assets, Handle};
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::perspective_camera::{PerspectiveCamera, ViewSize};
use kooch_ecs::query::Query;
use kooch_render::material::shader::{Shader, ShaderKind};

use crate::component::Planet;
use crate::draw::Blocks;
use crate::roster::{Roster, Signature};
use crate::{
    BARE_SPHERE, BlockKey, BlockPool, DEFAULT_CAPACITY, Limits, MeshedBlock, Planets, Residency,
    Shape, Terrain, View, select,
};

/// Blocks every planet in view gets before the others compete for the rest of the pool.
const FLOOR: u32 = 64;

/// What the planets cost this frame, for whatever shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlanetStats {
    pub resident: u32,
    pub capacity: u32,
    /// The worst error the pool left, in pixels.
    pub worst_px: f32,
    pub waiting: u32,
    pub in_flight: u32,
    pub meshed: u64,
    pub triangles: u64,
    /// The last batch's GPU cost; `None` without timestamps.
    pub gpu: Option<crate::GpuTimes>,
}

/// Everything the terrain keeps between frames.
pub struct PlanetTerrain {
    pool: BlockPool,
    residency: Residency,
    wanted: Vec<BlockKey>,
    planets: Planets,
    roster: Roster,
    terrain: Option<Terrain>,
    /// Set when the GPU side could not be built, so it is not tried every frame.
    broken: bool,
    meshed: Vec<MeshedBlock>,
    pub(crate) blocks: Blocks,
}

impl Default for PlanetTerrain {
    fn default() -> Self {
        Self {
            pool: BlockPool::new(DEFAULT_CAPACITY),
            residency: Residency::default(),
            wanted: Vec::new(),
            planets: Planets::default(),
            roster: Roster::default(),
            terrain: None,
            broken: false,
            meshed: Vec::new(),
            blocks: Blocks::default(),
        }
    }
}

/// One planet as this frame found it.
pub(crate) struct Found {
    pub entity: Entity,
    pub planet: Planet,
    pub world: Mat4,
}

/// Chooses, generates and draws every planet's blocks. Does nothing where nothing draws.
pub fn sync_planets(resources: &mut Resources) {
    let Some((device, queue)) = resources
        .get::<GpuContext>()
        .map(|gpu| (gpu.device().clone(), gpu.queue().clone()))
    else {
        return;
    };
    let found = gather(resources);
    let Some(mut state) = resources.remove::<PlanetTerrain>() else {
        return;
    };
    if state.terrain.is_none() && !state.broken {
        match Terrain::new(&device, &queue, DEFAULT_CAPACITY) {
            Ok(terrain) => state.terrain = Some(terrain),
            Err(why) => {
                tracing::error!(target: "kooch_planet", "planet terrain unavailable: {why}");
                state.broken = true;
            }
        }
    }
    let alive: Vec<Entity> = found.iter().map(|f| f.entity).collect();
    state.roster.retain(&alive);
    let mut placed: Vec<Option<usize>> = vec![None; state.roster.len() + found.len()];
    for (at, planet) in found.iter().enumerate() {
        let generator = planet.planet.generator;
        let (revision, shader) = load(resources, generator);
        let signature = Signature::new(&planet.planet, revision, shader.is_some());
        let (index, changed) = state.roster.place(planet.entity, signature);
        placed[index as usize] = Some(at);
        if changed && let Some(terrain) = &mut state.terrain {
            let source = shader.and_then(|handle| source(resources, handle));
            if generator.is_some() && shader.is_some() && source.is_none() {
                tracing::warn!(
                    target: "kooch_planet",
                    planet = %planet.entity,
                    "its generator is not a `planet` shader; drawing the bare sphere",
                );
            }
            let source = source.as_deref().unwrap_or(BARE_SPHERE);
            if let Err(why) = terrain.set_generator(&device, &state.pool, index, source) {
                tracing::error!(
                    target: "kooch_planet",
                    planet = %planet.entity,
                    "its generator does not compile; drawing the bare sphere:\n{why}",
                );
            }
        }
    }
    placed.truncate(state.roster.len());
    let (viewers, view) = viewers(resources);
    fill_planets(&mut state.planets, &placed, &found, &viewers);
    let worst = match viewers.is_empty() {
        true => {
            state.wanted.clear();
            0.0
        }
        false => select(
            &state.planets,
            &view,
            DEFAULT_CAPACITY,
            |_, _, _| true,
            &mut state.wanted,
        ),
    };
    state.pool.apply(&state.wanted, &mut state.residency);
    let mut meshed = std::mem::take(&mut state.meshed);
    if let Some(terrain) = &mut state.terrain {
        terrain.update(
            &device,
            &queue,
            &state.pool,
            &state.planets,
            &state.residency,
            &mut meshed,
        );
    }
    let mut blocks = std::mem::take(&mut state.blocks);
    blocks.hide(resources, &state.residency);
    blocks.draw(resources, &placed, &found, &mut meshed);
    blocks.follow(resources, &placed, &found);
    let stats = PlanetStats {
        resident: state.pool.resident(),
        capacity: state.pool.capacity(),
        worst_px: worst as f32,
        waiting: state.terrain.as_ref().map_or(0, |t| t.waiting() as u32),
        in_flight: state.terrain.as_ref().map_or(0, |t| t.in_flight() as u32),
        meshed: state.terrain.as_ref().map_or(0, Terrain::meshed),
        triangles: blocks.triangles(),
        gpu: state.terrain.as_ref().and_then(Terrain::times),
    };
    state.blocks = blocks;
    state.meshed = meshed;
    resources.insert(state);
    resources.insert(stats);
}

fn gather(resources: &Resources) -> Vec<Found> {
    let mut found = Vec::new();
    Query::<(Entity, &Planet, &GlobalTransform)>::new(resources).for_each(
        |(entity, planet, transform)| {
            found.push(Found {
                entity,
                planet: planet.clone(),
                world: transform.matrix,
            });
        },
    );
    found
}

/// The generator's reload count, and its shader once loaded.
fn load(resources: &mut Resources, guid: Option<Guid>) -> (u64, Option<Handle<Shader>>) {
    let Some(guid) = guid else {
        return (0, None);
    };
    let revision = resources
        .get::<ReloadedAssets>()
        .map_or(0, |reloaded| reloaded.revision(guid));
    let Some(mut server) = resources.remove::<AssetServer>() else {
        return (revision, None);
    };
    let handle = server.load_by_guid::<Shader>(guid, resources).ok();
    resources.insert(server);
    let loaded = handle.filter(|handle| {
        resources
            .get::<Assets<Shader>>()
            .is_some_and(|shaders| shaders.get(*handle).is_some())
    });
    (revision, loaded)
}

/// The generator's source, when it is a `planet` shader.
fn source(resources: &Resources, handle: Handle<Shader>) -> Option<String> {
    let shader = resources.get::<Assets<Shader>>()?.get(handle)?;
    (shader.kind == ShaderKind::Planet).then(|| shader.source.clone())
}

/// Every active camera's eye, and the most demanding lens among them.
fn viewers(resources: &Resources) -> (Vec<DVec3>, View) {
    let mut eyes = Vec::new();
    let mut fov = f32::MAX;
    Query::<(&PerspectiveCamera, &GlobalTransform)>::new(resources).for_each(
        |(camera, transform)| {
            if camera.active {
                eyes.push(transform.translation().as_dvec3());
                fov = fov.min(camera.fov.max(1.0));
            }
        },
    );
    let height = resources
        .get::<ViewSize>()
        .map(|size| size.0.y)
        .or_else(|| resources.get::<GpuContext>().map(|gpu| gpu.size().1))
        .unwrap_or(720)
        .max(1);
    (
        eyes,
        View::new(f64::from(height), f64::from(fov.min(179.0)).to_radians()),
    )
}

/// One row per roster index; a free index is a planet nobody sees, so it holds nothing.
fn fill_planets(planets: &mut Planets, placed: &[Option<usize>], found: &[Found], eyes: &[DVec3]) {
    planets.clear();
    let nowhere = vec![DVec3::splat(f64::INFINITY); eyes.len()];
    let mut local = Vec::with_capacity(eyes.len());
    for at in placed {
        let limits = Limits {
            floor: FLOOR,
            ceiling: DEFAULT_CAPACITY,
        };
        let Some(planet) = at.and_then(|at| found.get(at)) else {
            planets.push(Shape::from(&Planet::default()), &nowhere, limits);
            continue;
        };
        let to_local = planet.world.as_dmat4().inverse();
        local.clear();
        local.extend(eyes.iter().map(|eye| to_local.transform_point3(*eye)));
        let local = match to_local.is_finite() {
            true => &local[..],
            false => &nowhere[..],
        };
        planets.push(Shape::from(&planet.planet), local, limits);
    }
}

impl From<&Planet> for Shape {
    /// Clamped where a zero would stall the octree: a zero voxel never stops splitting.
    fn from(planet: &Planet) -> Self {
        Self {
            radius: f64::from(planet.radius.max(0.0)),
            relief: f64::from(planet.relief.max(0.0)),
            depth: f64::from(planet.depth.max(0.0)),
            voxel: f64::from(planet.voxel.max(0.01)),
        }
    }
}
