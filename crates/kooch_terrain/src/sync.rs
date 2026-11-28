//! Each frame: which blocks every planet holds for the cameras, the new ones sent to the GPU, and
//! the meshes that came back handed to [`draw`](crate::draw).
//!
//! 🔴 Runs where the frame is drawn, never where it is simulated: the editor's remote host has no
//! GPU, so the editor draws a playing game's planets itself, as it does a block's mesh.

mod ray;
mod stack;
mod twin;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::{DVec3, Mat4};
use kooch_core::frame_pacing::{FramePace, FrameRequest};
use kooch_core::gpu::GpuContext;
use kooch_core::resource::Resources;
use kooch_ecs::Entity;
use kooch_ecs::hierarchy::GlobalTransform;
use kooch_ecs::perspective_camera::{PerspectiveCamera, ViewSize};
use kooch_ecs::query::Query;
use kooch_render::meshlet::GpuMeshes;

use crate::component::Terrain;
use crate::draw::{Blocks, Frame};
use crate::eval::Generator;
use crate::lod::{Base, root_level};
use crate::rerun::{Inputs, Rerun};
use crate::roster::{Roster, Signature};
use crate::sight::Cone;
use crate::stamp::Stack;
pub use ray::{GroundHit, ground_hits};
use twin::{crosses, load, set_generator, source};

use crate::{
    BARE_SPHERE, BlockKey, BlockPool, CountedBlock, DEFAULT_CAPACITY, Leaves, Limits, Part,
    Pipeline, Planets, Residency, Shape, View, choose_lods,
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
    /// How many times closer than its ring allows the coarsest kept block sits; over 1 means the
    /// pool ran out before the ground near the camera reached its level.
    pub worst: f32,
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
    /// Shared, so the ground's workers mesh from it off the main thread.
    pub(crate) generators: Vec<Option<Arc<Generator>>>,
    /// Per planet index, its generator's WGSL, which the ground's GPU pass compiles.
    pub(crate) sources: Vec<Option<Arc<str>>>,
    /// Per planet index, bumped whenever its generator is replaced.
    pub(crate) twins: Vec<u32>,
    /// Per planet index, its stamps as the CPU twin and the GPU apply them (#1954).
    pub(crate) stacks: Vec<Arc<Stack>>,
    frame: u32,
    planets: Planets,
    pub(crate) roster: Roster,
    terrain: Option<Pipeline>,
    /// Set when the GPU side could not be built, so it is not tried every frame.
    broken: bool,
    meshed: Vec<CountedBlock>,
    rerun: Rerun,
    pub(crate) blocks: Blocks,
}

impl PlanetTerrain {
    /// The base shape `entity`'s generator picked, once its generator is loaded.
    pub fn base(&self, entity: Entity) -> Option<Base> {
        let index = self.roster.index_of(entity)?;
        let source = self.sources.get(index as usize)?.as_deref()?;
        Some(Base::of(source))
    }
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
            sources: Vec::new(),
            twins: Vec::new(),
            stacks: Vec::new(),
            frame: 0,
            planets: Planets::default(),
            roster: Roster::default(),
            terrain: None,
            broken: false,
            meshed: Vec::new(),
            rerun: Rerun::default(),
            blocks: Blocks::default(),
        }
    }
}

/// One planet as this frame found it.
pub(crate) struct Found {
    pub entity: Entity,
    pub planet: Terrain,
    pub world: Mat4,
    /// The centre in f64 metres from `cell`'s corner: `world` is relative to the origin, narrowed.
    pub at: DVec3,
    /// The cell `at` is measured from.
    pub cell: kooch_ecs::hierarchy::GridCell,
}

/// Chooses, generates and draws every planet's blocks. Does nothing where nothing draws.
pub fn sync_planets(resources: &mut Resources) {
    // The generators are read without a GPU too: the remote host simulates, and its ground
    // colliders are meshed from them (#1949).
    let gpu = resources
        .get::<GpuContext>()
        .map(|gpu| (gpu.device().clone(), gpu.queue().clone()));
    let found = {
        profiling::scope!("planet: gather");
        gather(resources)
    };
    let Some(mut state) = resources.remove::<PlanetTerrain>() else {
        return;
    };
    if let Some((device, queue)) = &gpu
        && state.terrain.is_none()
        && !state.broken
    {
        match Pipeline::new(device, queue, DEFAULT_CAPACITY) {
            Ok(terrain) => state.terrain = Some(terrain),
            Err(why) => {
                tracing::error!(target: "kooch_terrain", "planet terrain unavailable: {why}");
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
        if changed {
            let source = shader.and_then(|handle| source(resources, handle));
            if generator.is_some() && shader.is_some() && source.is_none() {
                tracing::warn!(
                    target: "kooch_terrain",
                    planet = %planet.entity,
                    "its generator is not a `planet` shader; drawing the bare sphere",
                );
            }
            let mut source = source.as_deref().unwrap_or(BARE_SPHERE);
            if let (Some(terrain), Some((device, _))) = (&mut state.terrain, &gpu)
                && let Err(why) = terrain.set_generator(device, &state.pool, index, source)
            {
                tracing::error!(
                    target: "kooch_terrain",
                    planet = %planet.entity,
                    "its generator does not compile; drawing the bare sphere:\n{why}",
                );
                source = BARE_SPHERE;
            }
            set_generator(&mut state, index, source);
        }
    }
    placed.truncate(state.roster.len());
    stack::sync_stacks(&mut state, &placed, &found);
    let Some((device, queue)) = gpu else {
        resources.insert(state);
        return;
    };
    let (viewers, cones, view) = viewers(resources);
    // The pool's bytes, not its block count, are what runs out first on dense relief (#1968).
    let budget = resources
        .get::<GpuMeshes>()
        .and_then(GpuMeshes::usage)
        .map_or(DEFAULT_CAPACITY, |usage| {
            crate::budget::blocks_within(state.pool.resident(), usage, DEFAULT_CAPACITY)
        });
    fill_planets(
        &mut state.planets,
        &placed,
        &found,
        &state.sources,
        &viewers,
        &cones,
    );
    let revisions: Vec<u32> = (0..state.planets.len() as u32)
        .map(|p| state.terrain.as_ref().map_or(0, |t| t.revision(p)))
        .collect();
    let inputs = Inputs {
        planets: &state.planets,
        view: &view,
        budget,
        revisions: &revisions,
    };
    let due = viewers.is_empty() || state.rerun.due(&inputs);
    let worst = match due {
        false => state.rerun.worst,
        true => {
            profiling::scope!("planet: choose lods");
            let mut deferred = false;
            let worst = match viewers.is_empty() {
                true => {
                    state.wanted.clear();
                    0.0
                }
                false => {
                    let started = Instant::now();
                    choose_lods(
                        &state.planets,
                        &view,
                        budget,
                        |key, min, max| {
                            let revision =
                                state.terrain.as_ref().map_or(0, |t| t.revision(key.planet));
                            let frame = state.frame;
                            if let Some(known) = state.known.get_mut(&key)
                                && known.revision == revision
                            {
                                known.seen = frame;
                                return known.surface;
                            }
                            if started.elapsed() > RANGE_BUDGET {
                                deferred = true;
                                return true;
                            }
                            profiling::scope!("planet: bound");
                            let twin = (&state.generators[..], &state.stacks[..]);
                            let surface = crosses(twin, &state.planets, key, min, max);
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
            let inputs = Inputs {
                planets: &state.planets,
                view: &view,
                budget,
                revisions: &revisions,
            };
            state.rerun.ran(&inputs, deferred, worst);
            // Only a selection refreshes `seen`, so a kept one must not age what it last met.
            let frame = state.frame;
            state
                .known
                .retain(|_, known| frame.wrapping_sub(known.seen) < KNOWN_GRACE);
            state.frame = frame.wrapping_add(1);
            worst
        }
    };
    {
        profiling::scope!("planet: pool");
        // Applying the same `wanted` again is a no-op: nothing leaves, and what did not fit still
        // does not, so the pool and its split set only change after a choice.
        match due {
            true => {
                state.pool.apply(&state.wanted, &mut state.residency);
                // After the pool changes, so the meshes see this frame's leaves; the next choice's
                // hysteresis reads the same set, as nothing moves the pool in between.
                fill_split(&mut state.split, &state.pool, &state.planets);
            }
            false => state.residency.clear(),
        }
    }
    let mut meshed = std::mem::take(&mut state.meshed);
    if let Some(terrain) = &mut state.terrain {
        profiling::scope!("planet: generate and count");
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
    // Empty means every mesh: a generated block counts all of them in the same batch.
    let surfaced: HashSet<BlockKey> = meshed
        .iter()
        .filter(|b| b.sizes.meshlets != 0)
        .map(|b| b.key)
        .collect();
    for block in meshed.iter().filter(|b| {
        b.part == Part::Interior
            && b.sizes.meshlets == 0
            && b.key.level == 0
            && !surfaced.contains(&b.key)
    }) {
        let known = Known {
            revision: block.revision,
            seen: state.frame,
            surface: false,
        };
        if state
            .known
            .insert(block.key, known)
            .is_none_or(|old| old.surface)
        {
            state.rerun.force();
        }
    }
    let mut blocks = std::mem::take(&mut state.blocks);
    if !state.residency.added.is_empty() || !state.residency.removed.is_empty() {
        blocks.unsettle();
    }
    let counting = meshed.iter().map(|b| b.epoch);
    let frontier = state.terrain.as_ref().map_or(u64::MAX, Pipeline::frontier);
    {
        profiling::scope!("planet: swap");
        blocks.swap(
            resources,
            &state.pool,
            &state.planets,
            counting.fold(frontier, u64::min),
        );
    }
    match &mut state.terrain {
        Some(terrain) => {
            let frame = Frame {
                device: &device,
                queue: &queue,
                pool: &state.pool,
                placed: &placed,
                found: &found,
            };
            profiling::scope!("planet: write");
            blocks.draw(resources, &frame, terrain, &mut meshed);
        }
        None => meshed.clear(),
    }
    {
        profiling::scope!("planet: submit");
        blocks.submit(resources, &placed, &found);
    }
    let stats = PlanetStats {
        resident: state.pool.resident(),
        capacity: state.pool.capacity(),
        budget,
        worst: worst as f32,
        waiting: state.terrain.as_ref().map_or(0, |t| t.waiting() as u32),
        in_flight: state.terrain.as_ref().map_or(0, |t| t.in_flight() as u32),
        meshed: state.terrain.as_ref().map_or(0, Pipeline::meshed),
        triangles: blocks.triangles(),
        meshlets: blocks.meshlets(),
        gpu: state.terrain.as_ref().and_then(Pipeline::times),
    };
    // An idle editor sleeps between frames, and a level change spread over sleeping frames shows
    // its seams half done (#1999). Wakes only while meshing; a game has no request and spins anyway.
    let meshing = state
        .terrain
        .as_ref()
        .is_some_and(|t| t.frontier() != u64::MAX);
    if meshing || !blocks.settled() {
        FrameRequest::raise(resources, FramePace::Continuous);
    }
    state.blocks = blocks;
    state.meshed = meshed;
    resources.insert(state);
    resources.insert(stats);
}

pub(crate) fn gather(resources: &Resources) -> Vec<Found> {
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
    Query::<(Entity, &Terrain, &GlobalTransform)>::new(resources).for_each(
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
    let view = View::new(f64::from(height.max(1)), f64::from(fov).to_radians());
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
    sources: &[Option<Arc<str>>],
    eyes: &[DVec3],
    cones: &[Cone],
) {
    planets.clear();
    let nowhere = vec![DVec3::splat(f64::INFINITY); eyes.len()];
    let mut local = Vec::with_capacity(eyes.len());
    let mut turned = Vec::with_capacity(cones.len());
    for (index, at) in placed.iter().enumerate() {
        let limits = Limits {
            floor: FLOOR,
            ceiling: DEFAULT_CAPACITY,
        };
        let Some(planet) = at.and_then(|at| found.get(at)) else {
            planets.push(
                Shape::new(&Terrain::default(), Base::Sphere),
                &nowhere,
                limits,
            );
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
        let base = sources
            .get(index)
            .and_then(Option::as_deref)
            .map_or(Base::Sphere, Base::of);
        planets.push_seen(Shape::new(&planet.planet, base), local, &turned, limits);
    }
}

impl Shape {
    /// `planet` measured from `base`, its generator's, grown to hold its stamps. Clamped where a
    /// zero would stall the octree: a zero voxel never stops splitting.
    pub fn new(planet: &Terrain, base: Base) -> Self {
        Self {
            base,
            radius: f64::from(planet.radius.max(0.0)),
            extent: f64::from(planet.extent.max(0.0)),
            relief: f64::from(planet.relief.max(0.0)),
            depth: f64::from(planet.depth.max(0.0)),
            voxel: f64::from(planet.voxel.max(0.01)),
            lod_distance: f64::from(planet.lod_distance.max(0.0)),
        }
        .hold(&planet.stamps)
    }
}
