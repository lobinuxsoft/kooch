//! Colliders for the ground near bodies (#1949): every level-0 block within reach of a moving body
//! is generated on the GPU, read back and meshed on a worker thread; a planet's blocks reach physics
//! welded into one static trimesh ([`super::patch`]). Nothing else of a planet collides, so the cost
//! follows the bodies.
//!
//! The patch is a body straight in the solver, never an entity, like the drawn blocks (#1991). It is
//! placed from its anchor block's cell, not as its planet's child: a child only exists narrowed to f32 from the
//! origin, half a metre off on an Earth-sized planet.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use glam::{DVec3, IVec3, Mat3};
use kooch_core::resource::Resources;
use kooch_core::run_state::Playing;
use kooch_ecs::component::{Component, ComponentRegistry};
use kooch_ecs::hierarchy::{GlobalTransform, GridCell, WorldOrigin};
use kooch_ecs::transform::Transform;
use kooch_ecs::{Entity, Reflect};
use kooch_physics::components::KIND_STATIC;
use kooch_physics::{PhysicsBody, PhysicsHolds, PhysicsWorld};

use super::gpu::{GroundGpu, Order};
use super::mesh_block;
use super::patch::{self, Merged, Patch};
use crate::lod::{Base, bounds, voxel_size};
use crate::sync::{PlanetTerrain, gather};
use crate::{BLOCK_SAMPLES, BlockKey, Shape};

/// Metres of ground every moving body asks for without a [`GroundInvoker`].
const DEFAULT_REACH: f32 = 4.0;

/// Metres around a body whose ground must exist before it simulates. Smaller than the reach: the
/// rest is lookahead, and a body held for every block entering its reach stutters as it rolls.
const HOLD: f64 = 1.5;

/// Overrides how much ground around a body collides. Every dynamic or kinematic body asks for
/// [`DEFAULT_REACH`] without one; put it on a fast body, or on anything that needs ground and has
/// no body.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(category = "Physics")]
pub struct GroundInvoker {
    /// Metres around the body whose ground collides before the body gets there, rounded up to
    /// whole blocks: the block it is in and this many rings of neighbours are built ahead, and
    /// released once it is a ring further than that.
    pub reach: f32,
}

impl Default for GroundInvoker {
    fn default() -> Self {
        Self {
            reach: DEFAULT_REACH,
        }
    }
}

impl Component for GroundInvoker {}

pub(super) struct Held {
    /// New on every build, so a viewer knows which triangles it has not seen yet.
    pub(super) id: u64,
    /// In its planet's patch in the solver.
    pub(super) merged: bool,
    twin: u32,
    /// The last frame a body was within its keeping ring; any other frame drops it.
    seen: u32,
    pub(super) place: Place,
    /// Its triangles, the patch's source; `None` for a block with no surface, remembered so it is
    /// never generated again.
    pub(super) wire: Option<Arc<GroundWire>>,
}

/// Where a block sits: its minimum corner, and its side in the planet's scale.
#[derive(Clone, Copy, Default)]
pub(super) struct Place {
    pub(super) transform: Transform,
    pub(super) cell: GridCell,
    pub(super) side: f32,
}

/// A block's triangles from its corner, the planet's scale applied.
pub struct GroundWire {
    pub vertices: Vec<glam::Vec3>,
    pub indices: Vec<[u32; 3]>,
}

/// A block a worker finished; `mesh` is `None` when it holds no surface.
struct Done {
    key: BlockKey,
    twin: u32,
    epoch: u32,
    mesh: Option<Arc<GroundWire>>,
    place: Place,
}

/// The ground blocks a body reached, by key.
#[derive(Default)]
pub struct GroundColliders {
    pub(super) held: HashMap<BlockKey, Held>,
    /// Blocks on the GPU or a worker now.
    pub(super) pending: HashMap<BlockKey, Place>,
    next_id: u64,
    done: Arc<Mutex<Vec<Done>>>,
    pub(super) patches: HashMap<u32, Patch>,
    pub(super) merged: Arc<Mutex<Vec<Merged>>>,
    /// Bumped when Play stops, so a worker finishing late adds nothing.
    pub(super) epoch: u32,
    gpu: Option<GroundGpu<Ticket>>,
    /// Set once acquiring a device failed, so it is not retried every frame.
    no_gpu: bool,
    frame: u32,
}

/// What travels with a block through the GPU and back.
struct Ticket {
    key: BlockKey,
    twin: u32,
    epoch: u32,
    voxel: f32,
    transform: Transform,
    cell: GridCell,
    asked: Instant,
}

/// A body asking for ground, from the origin.
struct Body {
    entity: Entity,
    at: DVec3,
    reach: f64,
}

/// A block to mesh, with what a worker needs for it.
struct Want {
    key: BlockKey,
    source: Arc<str>,
    shape: Shape,
    /// Its planet's CPU generator revision; a new one rebuilds the block.
    twin: u32,
    /// From the nearest body, in metres: what goes to the workers first.
    distance: f64,
    transform: Transform,
    cell: GridCell,
}

/// What the bodies reached this frame.
#[derive(Default)]
struct Reached {
    wants: Vec<Want>,
    /// Held blocks some body still keeps.
    seen: Vec<BlockKey>,
    /// Bodies with ground still to mesh around them: they wait instead of falling through.
    waiting: Vec<Entity>,
}

/// Keeps a collider on every block a body reaches, drops the ones nobody does, and holds the bodies
/// whose ground is not there yet.
pub fn sync_ground(resources: &mut Resources) {
    profiling::scope!("planet: ground");
    let Some(mut ground) = resources.remove::<GroundColliders>() else {
        return;
    };
    ground.frame = ground.frame.wrapping_add(1);
    let frame = ground.frame;
    let done = std::mem::take(&mut *ground.done.lock().unwrap_or_else(PoisonError::into_inner));
    let reached = reach(resources, &ground);
    for key in &reached.seen {
        if let Some(held) = ground.held.get_mut(key) {
            held.seen = frame;
        }
    }
    dispatch(resources, &mut ground, reached.wants);
    let mut gone: Vec<u32> = Vec::new();
    ground.held.retain(|key, held| {
        let keep = held.seen == frame;
        if !keep && held.wire.is_some() {
            gone.push(key.planet);
        }
        keep
    });
    for planet in gone {
        ground.patches.entry(planet).or_default().dirty = true;
    }
    publish(&mut ground, done);
    patch::merge(&mut ground);
    let origin = WorldOrigin::get(resources);
    if let Some(world) = resources.get_mut::<PhysicsWorld>() {
        let backend = world.backend_mut();
        patch::revive(backend, &mut ground);
        patch::swap(backend, &origin, &mut ground);
    }
    resources.insert(ground);
    if !reached.waiting.is_empty() {
        tracing::trace!(target: "kooch_terrain", bodies = reached.waiting.len(), "waiting for ground");
    }
    if let Some(holds) = resources.get_mut::<PhysicsHolds>() {
        holds.set(reached.waiting);
    }
}

fn reach(resources: &Resources, ground: &GroundColliders) -> Reached {
    let mut reached = Reached::default();
    let bodies = bodies(resources);
    if bodies.is_empty() {
        return reached;
    }
    let Some(state) = resources.get::<PlanetTerrain>() else {
        return reached;
    };
    let origin = WorldOrigin::get(resources);
    let mut wants: HashMap<BlockKey, Want> = HashMap::new();
    for planet in gather(resources) {
        let Some(index) = state.roster.index_of(planet.entity) else {
            continue;
        };
        let Some(Some(source)) = state.sources.get(index as usize) else {
            continue;
        };
        let twin = state.twins.get(index as usize).copied().unwrap_or(0);
        let shape = Shape::new(&planet.planet, Base::of(source));
        let basis = Mat3::from_mat4(planet.world).as_dmat3();
        if !(basis.determinant().abs() > 1e-12) {
            continue;
        }
        let to_local = basis.inverse();
        // The planet's scale travels on the block: its lengths are in the planet's local units.
        let (scale, rotation, _) = planet.world.to_scale_rotation_translation();
        let centre = planet.at - origin.from(Some(&planet.cell));
        let side = voxel_size(&shape, 0) * f64::from(BLOCK_SAMPLES);
        for body in &bodies {
            let local = to_local * (body.at - centre);
            let ring = (body.reach / side).ceil().max(1.0) as i32;
            let extent = f64::from(ring + 1) * side;
            let altitude = match shape.base {
                Base::Sphere => local.length() - shape.radius,
                Base::Plane if local.x.abs().max(local.z.abs()) > shape.extent + extent => continue,
                Base::Plane => local.y,
            };
            if altitude > shape.relief + extent || altitude < -shape.depth - extent {
                continue;
            }
            let home = (local / side).floor().as_ivec3();
            let mut ready = true;
            // One ring past the built ones is kept, so a body pacing a block's edge rebuilds nothing.
            for offset in cube(ring + 1) {
                let key = BlockKey {
                    planet: index,
                    level: 0,
                    cell: home + offset,
                };
                let held = ground.held.get(&key);
                let near = offset.abs().max_element() <= ring;
                let fresh = held.is_some_and(|held| held.twin == twin);
                if held.is_some() && (fresh || !near) {
                    reached.seen.push(key);
                }
                if !near {
                    continue;
                }
                let (min, max) = bounds(&shape, key);
                let distance = (local.clamp(min, max) - local).length();
                // A stale block still collides while it is rebuilt; a fresh one only once merged.
                let solid = held.is_some_and(|held| held.merged || held.wire.is_none());
                ready &= solid || distance > HOLD;
                if fresh {
                    continue;
                }
                if let Some(want) = wants.get_mut(&key) {
                    want.distance = want.distance.min(distance);
                    continue;
                }
                let transform = Transform {
                    position: planet.at + basis * min,
                    rotation,
                    scale,
                };
                let want = Want {
                    key,
                    source: Arc::clone(source),
                    shape,
                    twin,
                    distance,
                    transform,
                    cell: planet.cell,
                };
                wants.insert(key, want);
            }
            if !ready {
                reached.waiting.push(body.entity);
            }
        }
    }
    reached.wants = wants.into_values().collect();
    reached
        .wants
        .sort_by(|a, b| a.distance.total_cmp(&b.distance));
    reached
}

/// Every moving body and every invoker, with the ground it asks for.
fn bodies(resources: &Resources) -> Vec<Body> {
    let Some(registry) = resources.get::<ComponentRegistry>() else {
        return Vec::new();
    };
    let Some(globals) = registry.get_cpu::<GlobalTransform>() else {
        return Vec::new();
    };
    let invokers = registry.get_cpu::<GroundInvoker>();
    let physics = registry.get_cpu::<PhysicsBody>();
    let moving = |entity: Entity| {
        physics
            .as_ref()
            .and_then(|physics| physics.get(entity))
            .is_some_and(|body| body.kind != KIND_STATIC)
    };
    let body = |entity: Entity| {
        let reach = invokers
            .as_ref()
            .and_then(|invokers| invokers.get(entity))
            .map_or(DEFAULT_REACH, |invoker| invoker.reach);
        Some(Body {
            entity,
            at: globals.get(entity)?.translation().as_dvec3(),
            reach: f64::from(reach.max(0.0)),
        })
    };
    let mut bodies: Vec<Body> = Vec::new();
    if let Some(physics) = &physics {
        let moved = physics.iter().map(|(entity, _)| *entity);
        bodies.extend(moved.filter(|e| moving(*e)).filter_map(body));
    }
    if let Some(invokers) = &invokers {
        let asked = invokers.iter().map(|(entity, _)| *entity);
        bodies.extend(asked.filter(|e| !moving(*e)).filter_map(body));
    }
    bodies
}

/// Every offset within `ring` blocks on each axis.
fn cube(ring: i32) -> impl Iterator<Item = IVec3> {
    (-ring..=ring).flat_map(move |z| {
        (-ring..=ring).flat_map(move |y| (-ring..=ring).map(move |x| IVec3::new(x, y, z)))
    })
}

/// Collects the tiles the GPU read back, sends their meshing to workers, and hands the GPU the
/// nearest wanted blocks it has room for.
fn dispatch(resources: &Resources, ground: &mut GroundColliders, wants: Vec<Want>) {
    let frame = ground.frame;
    for want in &wants {
        if let Some(held) = ground.held.get_mut(&want.key) {
            held.seen = frame;
        }
    }
    if ground.gpu.is_none() && !ground.no_gpu && !wants.is_empty() {
        ground.gpu = GroundGpu::acquire(resources);
        ground.no_gpu = ground.gpu.is_none();
    }
    let Some(gpu) = &mut ground.gpu else {
        return;
    };
    for (ticket, samples) in gpu.collect() {
        let Some(samples) = samples else {
            // Lost by the device: asked again next frame.
            ground.pending.remove(&ticket.key);
            continue;
        };
        let done = Arc::clone(&ground.done);
        rayon::spawn(move || {
            let block = mesh(ticket, &samples);
            done.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(block);
        });
    }
    let free = gpu.free();
    let orders: Vec<Order<Ticket>> = wants
        .into_iter()
        .filter(|want| !ground.pending.contains_key(&want.key))
        .take(free)
        .map(|want| Order {
            key: want.key,
            shape: want.shape,
            twin: want.twin,
            source: want.source,
            ticket: Ticket {
                key: want.key,
                twin: want.twin,
                epoch: ground.epoch,
                voxel: voxel_size(&want.shape, 0) as f32,
                transform: want.transform,
                cell: want.cell,
                asked: Instant::now(),
            },
        })
        .collect();
    for order in &orders {
        ground.pending.insert(order.key, order.ticket.place());
    }
    for refused in gpu.submit(orders) {
        ground.pending.remove(&refused.key);
    }
}

/// On a worker: the block's trimesh from the samples the GPU generated.
fn mesh(ticket: Ticket, samples: &[f32]) -> Done {
    let started = Instant::now();
    let meshed = mesh_block(samples, ticket.voxel);
    let mesh = meshed;
    tracing::trace!(
        target: "kooch_terrain",
        key = ?ticket.key,
        triangles = mesh.indices.len(),
        mesh_us = started.elapsed().as_micros() as u64,
        total_us = ticket.asked.elapsed().as_micros() as u64,
        "ground block",
    );
    // A solver body has no scale, so the planet's is baked into the triangles.
    let scale = ticket.transform.scale;
    let mesh = (!mesh.indices.is_empty()).then(|| {
        Arc::new(GroundWire {
            vertices: mesh.vertices.into_iter().map(|v| v * scale).collect(),
            indices: mesh.indices,
        })
    });
    Done {
        key: ticket.key,
        twin: ticket.twin,
        epoch: ticket.epoch,
        mesh,
        place: ticket.place(),
    }
}

impl Ticket {
    fn place(&self) -> Place {
        Place {
            transform: self.transform,
            cell: self.cell,
            side: self.voxel * BLOCK_SAMPLES as f32,
        }
    }
}

/// Holds the finished blocks, and marks the planets whose surface they change.
fn publish(ground: &mut GroundColliders, done: Vec<Done>) {
    for block in done {
        if block.epoch != ground.epoch {
            continue;
        }
        ground.pending.remove(&block.key);
        let had = ground
            .held
            .get(&block.key)
            .is_some_and(|held| held.wire.is_some());
        if had || block.mesh.is_some() {
            ground.patches.entry(block.key.planet).or_default().dirty = true;
        }
        ground.next_id += 1;
        let held = Held {
            id: ground.next_id,
            merged: false,
            twin: block.twin,
            seen: ground.frame,
            place: block.place,
            wire: block.mesh,
        };
        ground.held.insert(block.key, held);
    }
}

/// Takes every patch out of the solver once Play stops: they are nobody's entities, so restoring
/// the scene does not remove them.
pub fn release_ground(resources: &mut Resources) {
    if Playing::is_playing(resources) {
        return;
    }
    let Some(mut ground) = resources.remove::<GroundColliders>() else {
        return;
    };
    if !ground.held.is_empty() || !ground.pending.is_empty() || !ground.patches.is_empty() {
        if let Some(world) = resources.get_mut::<PhysicsWorld>() {
            let backend = world.backend_mut();
            for body in ground.patches.values().filter_map(|patch| patch.body) {
                backend.remove_body(body);
            }
        }
        ground.epoch = ground.epoch.wrapping_add(1);
        ground.held.clear();
        ground.pending.clear();
        ground.patches.clear();
        ground
            .merged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        ground
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
    resources.insert(ground);
}
