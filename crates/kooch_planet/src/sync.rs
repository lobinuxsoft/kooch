//! Each frame: which blocks every planet holds for the cameras, the new ones sent to the GPU, and
//! the meshes that came back handed to [`draw`](crate::draw).
//!
//! 🔴 Runs where the frame is drawn, never where it is simulated: the editor's remote host has no
//! GPU, so the editor draws a playing game's planets itself, as it does a block's mesh.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

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
use kooch_render::meshlet::{GpuMeshes, MeshletLodSettings};

use crate::component::Planet;
use crate::draw::{Blocks, Frame};
use crate::eval::{Generator, may_cross};
use crate::roster::{Roster, Signature};
use crate::select::{root_level, voxel_size};
use crate::sight::Cone;
use crate::{
    BARE_SPHERE, BlockKey, BlockPool, CountedBlock, DEFAULT_CAPACITY, Leaves, Limits, Planets,
    Residency, Shape, Terrain, View, select,
};

/// Blocks every planet in view gets before the others compete for the rest of the pool.
const FLOOR: u32 = 64;

/// Frames a block's surface answer is remembered unasked: a parent that merges and splits back
/// while the viewer moves would otherwise bound, generate and count its children again.
const KNOWN_GRACE: u32 = 600;

/// CPU time a frame spends bounding blocks the selection has not met; the rest wait, holding a
/// slot meanwhile, so entering a new region never stalls a frame.
const RANGE_BUDGET: Duration = Duration::from_millis(1);

/// Whether a block can hold surface, as the generator's range or its count answered.
#[derive(Clone, Copy)]
struct Known {
    /// Its planet's revision when answered.
    revision: u32,
    /// The last frame the selection asked.
    seen: u32,
    surface: bool,
}

/// What the planets cost this frame, for whatever shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlanetStats {
    pub resident: u32,
    pub capacity: u32,
    /// Blocks the meshlet pool's bindings hold at the current cost per block; below `capacity`
    /// when the pool's bytes are the limit (#1968).
    pub budget: u32,
    /// The worst error the pool left, in pixels.
    pub worst_px: f32,
    pub waiting: u32,
    pub in_flight: u32,
    pub meshed: u64,
    pub triangles: u64,
    /// Over `triangles`, how full the mesher fills them (#1935).
    pub meshlets: u64,
    /// The last batch's GPU cost; `None` without timestamps.
    pub gpu: Option<crate::GpuTimes>,
}

/// Which active cameras the octree refines for. A game leaves it at `Every`; a host that draws a
/// camera only for itself, as an editor does, narrows it so that camera's detail is not paid twice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlanetViewers {
    #[default]
    Every,
    Only(Entity),
    Except(Entity),
}

impl PlanetViewers {
    fn admits(self, camera: Entity) -> bool {
        match self {
            Self::Every => true,
            Self::Only(only) => camera == only,
            Self::Except(except) => camera != except,
        }
    }
}

/// Everything the terrain keeps between frames.
pub struct PlanetTerrain {
    pool: BlockPool,
    residency: Residency,
    wanted: Vec<BlockKey>,
    /// Every block the pool held a descendant of last frame, for the selection's hysteresis.
    split: HashSet<BlockKey>,
    /// Blocks the generator bounded or the GPU counted. One with no surface takes no slot; what
    /// roaming leaves behind is dropped after [`KNOWN_GRACE`].
    known: HashMap<BlockKey, Known>,
    /// Per planet index, its generator on the CPU; `None` keeps every block a candidate.
    generators: Vec<Option<Generator>>,
    frame: u32,
    planets: Planets,
    roster: Roster,
    terrain: Option<Terrain>,
    /// Set when the GPU side could not be built, so it is not tried every frame.
    broken: bool,
    meshed: Vec<CountedBlock>,
    pub(crate) blocks: Blocks,
}

impl Default for PlanetTerrain {
    fn default() -> Self {
        Self {
            pool: BlockPool::new(DEFAULT_CAPACITY),
            residency: Residency::default(),
            wanted: Vec::new(),
            split: HashSet::new(),
            known: HashMap::new(),
            generators: Vec::new(),
            frame: 0,
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
    /// The centre in f64 metres from `cell`'s corner: `world` is relative to the origin, narrowed.
    pub at: DVec3,
    /// The cell `at` is measured from.
    pub cell: kooch_ecs::hierarchy::GridCell,
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
            let mut source = source.as_deref().unwrap_or(BARE_SPHERE);
            if let Err(why) = terrain.set_generator(&device, &state.pool, index, source) {
                tracing::error!(
                    target: "kooch_planet",
                    planet = %planet.entity,
                    "its generator does not compile; drawing the bare sphere:\n{why}",
                );
                source = BARE_SPHERE;
            }
            set_generator(&mut state.generators, index, source);
        }
    }
    placed.truncate(state.roster.len());
    let (viewers, cones, view) = viewers(resources);
    // The pool's bytes, not its block count, are what runs out first on dense relief (#1968).
    let budget = resources
        .get::<GpuMeshes>()
        .and_then(GpuMeshes::usage)
        .map_or(DEFAULT_CAPACITY, |usage| {
            crate::budget::blocks_within(state.pool.resident(), usage, DEFAULT_CAPACITY)
        });
    fill_planets(&mut state.planets, &placed, &found, &viewers, &cones);
    let worst = match viewers.is_empty() {
        true => {
            state.wanted.clear();
            0.0
        }
        false => {
            let started = Instant::now();
            select(
                &state.planets,
                &view,
                budget,
                |key, min, max| {
                    let revision = state.terrain.as_ref().map_or(0, |t| t.revision(key.planet));
                    let frame = state.frame;
                    if let Some(known) = state.known.get_mut(&key)
                        && known.revision == revision
                    {
                        known.seen = frame;
                        return known.surface;
                    }
                    if started.elapsed() > RANGE_BUDGET {
                        return true;
                    }
                    let surface = crosses(&state.generators, &state.planets, key, min, max);
                    let known = Known {
                        revision,
                        seen: frame,
                        surface,
                    };
                    state.known.insert(key, known);
                    surface
                },
                |key| state.split.contains(&key),
                &mut state.wanted,
            )
        }
    };
    let frame = state.frame;
    state
        .known
        .retain(|_, known| frame.wrapping_sub(known.seen) < KNOWN_GRACE);
    state.frame = frame.wrapping_add(1);
    state.pool.apply(&state.wanted, &mut state.residency);
    // After the pool changes, so the meshes see this frame's leaves; the next selection's
    // hysteresis reads the same set, as nothing moves the pool in between.
    fill_split(&mut state.split, &state.pool, &state.planets);
    let mut meshed = std::mem::take(&mut state.meshed);
    if let Some(terrain) = &mut state.terrain {
        let leaves = Leaves {
            pool: &state.pool,
            split: &state.split,
        };
        terrain.update(
            &device,
            &queue,
            leaves,
            &state.planets,
            &state.residency,
            &mut meshed,
        );
    }
    // Only the finest level's empty count is final: a coarser block samples every 2^level voxels
    // and misses a ridge thinner than that, and calling it empty would prune its children (#1945).
    for block in meshed
        .iter()
        .filter(|b| b.sizes.meshlets == 0 && b.key.level == 0)
    {
        let known = Known {
            revision: block.revision,
            seen: state.frame,
            surface: false,
        };
        state.known.insert(block.key, known);
    }
    let mut blocks = std::mem::take(&mut state.blocks);
    if !state.residency.added.is_empty() || !state.residency.removed.is_empty() {
        blocks.unsettle();
    }
    blocks.swap(resources, &state.pool, &state.planets);
    match &mut state.terrain {
        Some(terrain) => {
            let frame = Frame {
                device: &device,
                queue: &queue,
                pool: &state.pool,
                placed: &placed,
                found: &found,
            };
            blocks.draw(resources, &frame, terrain, &mut meshed);
        }
        None => meshed.clear(),
    }
    blocks.follow(resources, &placed, &found);
    let stats = PlanetStats {
        resident: state.pool.resident(),
        capacity: state.pool.capacity(),
        budget,
        worst_px: worst as f32,
        waiting: state.terrain.as_ref().map_or(0, |t| t.waiting() as u32),
        in_flight: state.terrain.as_ref().map_or(0, |t| t.in_flight() as u32),
        meshed: state.terrain.as_ref().map_or(0, Terrain::meshed),
        triangles: blocks.triangles(),
        meshlets: blocks.meshlets(),
        gpu: state.terrain.as_ref().and_then(Terrain::times),
    };
    state.blocks = blocks;
    state.meshed = meshed;
    resources.insert(state);
    resources.insert(stats);
}

/// The CPU twin of what the GPU now generates for planet `index`.
fn set_generator(generators: &mut Vec<Option<Generator>>, index: u32, source: &str) {
    let index = index as usize;
    if generators.len() <= index {
        generators.resize_with(index + 1, || None);
    }
    generators[index] = Generator::parse(source)
        .inspect_err(|why| tracing::debug!(target: "kooch_planet", "no CPU generator: {why}"))
        .ok();
}

/// Whether the generator leaves `key`'s tile a chance of holding surface. The tile reaches one
/// sample past the block on every side, so its border is bounded too.
fn crosses(
    generators: &[Option<Generator>],
    planets: &Planets,
    key: BlockKey,
    min: DVec3,
    max: DVec3,
) -> bool {
    let planet = key.planet as usize;
    let (Some(Some(generator)), Some(shape)) = (generators.get(planet), planets.shape.get(planet))
    else {
        return true;
    };
    let voxel = voxel_size(shape, key.level);
    let pad = DVec3::splat(voxel);
    may_cross(
        generator.range(shape, min - pad, max + pad, key.planet),
        voxel,
    )
}

fn gather(resources: &Resources) -> Vec<Found> {
    use kooch_ecs::hierarchy::GridCell;
    let origin = kooch_ecs::hierarchy::WorldOrigin::get(resources);
    let registry = resources.get::<kooch_ecs::component::ComponentRegistry>();
    let transforms = registry.and_then(|r| r.get_cpu::<kooch_ecs::transform::Transform>());
    let parents = registry.and_then(|r| r.get_cpu::<kooch_ecs::hierarchy::Parent>());
    let cells = registry.and_then(|r| r.get_cpu::<GridCell>());
    // A root's own f64 position in its cell; a child's only exists narrowed, from the origin's.
    let at = |entity: Entity, world: &Mat4| {
        let root = parents.as_ref().is_none_or(|p| p.get(entity).is_none());
        let own = transforms
            .as_ref()
            .and_then(|t| t.get(entity))
            .filter(|_| root);
        let cell = cells.as_ref().and_then(|c| c.get(entity)).copied();
        match own {
            Some(t) => (t.position, cell.unwrap_or_default()),
            None => {
                let (x, y, z) = (origin.cell.x, origin.cell.y, origin.cell.z);
                (world.w_axis.truncate().as_dvec3(), GridCell { x, y, z })
            }
        }
    };
    let mut found = Vec::new();
    Query::<(Entity, &Planet, &GlobalTransform)>::new(resources).for_each(
        |(entity, planet, transform)| {
            let (at, cell) = at(entity, &transform.matrix);
            found.push(Found {
                entity,
                planet: planet.clone(),
                world: transform.matrix,
                at,
                cell,
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

/// Every admitted active camera's eye, and the most demanding lens among them.
/// Each admitted camera's position and view cone, in world space, and the view to measure error by.
fn viewers(resources: &Resources) -> (Vec<DVec3>, Vec<Cone>, View) {
    let admitted = resources
        .get::<PlanetViewers>()
        .copied()
        .unwrap_or_default();
    let (width, height) = resources
        .get::<ViewSize>()
        .map(|size| (size.0.x, size.0.y))
        .or_else(|| resources.get::<GpuContext>().map(|gpu| gpu.size()))
        .unwrap_or((1280, 720));
    let aspect = f64::from(width.max(1)) / f64::from(height.max(1));
    let mut eyes = Vec::new();
    let mut cones = Vec::new();
    let mut fov = f32::MAX;
    Query::<(Entity, &PerspectiveCamera, &GlobalTransform)>::new(resources).for_each(
        |(entity, camera, transform)| {
            if camera.active && admitted.admits(entity) {
                let degrees = camera.fov.clamp(1.0, 179.0);
                let forward = -transform.matrix.z_axis.truncate().as_dvec3();
                eyes.push(transform.translation().as_dvec3());
                cones.push(Cone::new(forward, f64::from(degrees).to_radians(), aspect));
                fov = fov.min(degrees);
            }
        },
    );
    let target = resources
        .get::<MeshletLodSettings>()
        .map_or(MeshletLodSettings::default().planet_voxel_pixels, |lod| {
            lod.planet_voxel_pixels
        });
    let view = View::new(
        f64::from(height.max(1)),
        f64::from(fov).to_radians(),
        f64::from(target),
    );
    (eyes, cones, view)
}

/// Every block the pool holds a descendant of: what the selection split last frame.
fn fill_split(split: &mut HashSet<BlockKey>, pool: &BlockPool, planets: &Planets) {
    split.clear();
    for key in pool.keys() {
        let root = planets.shape.get(key.planet as usize).map_or(0, root_level);
        for up in key.ancestors(root) {
            if !split.insert(up) {
                break;
            }
        }
    }
}

/// One row per roster index; a free index is a planet nobody sees, so it holds nothing.
fn fill_planets(
    planets: &mut Planets,
    placed: &[Option<usize>],
    found: &[Found],
    eyes: &[DVec3],
    cones: &[Cone],
) {
    planets.clear();
    let nowhere = vec![DVec3::splat(f64::INFINITY); eyes.len()];
    let mut local = Vec::with_capacity(eyes.len());
    let mut turned = Vec::with_capacity(cones.len());
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
        turned.clear();
        turned.extend(cones.iter().map(|cone| Cone {
            axis: to_local.transform_vector3(cone.axis).normalize_or_zero(),
            ..*cone
        }));
        let local = match to_local.is_finite() {
            true => &local[..],
            false => &nowhere[..],
        };
        planets.push_seen(Shape::from(&planet.planet), local, &turned, limits);
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
