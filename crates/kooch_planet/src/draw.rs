//! The entities that draw the blocks, one per draw index. A block that changes level stays on
//! screen until what replaces it has its mesh, as Zylann's octree does: see [`swap`].
//!
//! A block's mesh is counted, asked of the meshlet pool, granted the frame after and written in
//! place by the GPU, under a GUID of its own per ask, so a stale grant never meets a newer ask.

mod swap;

use std::collections::HashMap;

use glam::{DVec3, Mat4, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::query::Query;
use kooch_ecs::{Commands, Entity, MeshRenderer, Transform};
use kooch_render::meshlet::{GpuMeshes, MeshBounds};

use crate::component::PlanetBlock;
use crate::sync::Found;
use crate::{BLOCK_SAMPLES, BlockKey, BlockPool, CountedBlock, Terrain, WriteJob};

/// A mesh asked of the pool and not granted yet.
#[derive(Clone, Copy)]
struct Asked {
    mesh: Guid,
    block: CountedBlock,
}

/// Per draw index, as parallel arrays. An index outlives its block's pool slot while it lingers.
#[derive(Default)]
pub(crate) struct Blocks {
    entity: Vec<Option<Entity>>,
    key: Vec<Option<BlockKey>>,
    /// The block's corner in its planet's space.
    origin: Vec<DVec3>,
    /// About how many: counted from triangle bytes, which round up per meshlet.
    triangles: Vec<u32>,
    /// The mesh on screen.
    mesh: Vec<Option<Guid>>,
    /// Written and waiting for its swap.
    staged: Vec<Option<Guid>>,
    asked: Vec<Option<Asked>>,
    /// Whether the block's mesh is in hand: written, or nothing to draw.
    built: Vec<bool>,
    shown: Vec<bool>,
    /// The frame the block left the pool, while it lingers on screen.
    left: Vec<Option<u32>>,
    free: Vec<u32>,
    /// Every block holding an index. Streaming bookkeeping, CPU only.
    of_key: HashMap<BlockKey, u32>,
    /// Bumped per ask; half of every mesh GUID.
    asks: u32,
    frame: u32,
    /// Nothing waits and nothing lingers, so [`Blocks::swap`] has nothing to do.
    settled: bool,
    /// Per roster index: the matrix and material its blocks were last drawn with.
    world: Vec<Mat4>,
    material: Vec<Option<Guid>>,
}

/// What [`Blocks::draw`] reads of the frame.
pub(crate) struct Frame<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub pool: &'a BlockPool,
    pub placed: &'a [Option<usize>],
    pub found: &'a [Found],
}

impl Blocks {
    /// The pool changed: something may now wait or linger.
    pub(crate) fn unsettle(&mut self) {
        self.settled = false;
    }

    /// Writes the meshes the pool granted since last frame, then asks for the ones just counted.
    /// A block already on screen takes its new mesh at once; a new one waits hidden for
    /// [`Blocks::swap`], which needs its transform propagated first.
    pub(crate) fn draw(
        &mut self,
        resources: &mut Resources,
        frame: &Frame,
        terrain: &mut Terrain,
        counted: &mut Vec<CountedBlock>,
    ) {
        let Some(mut meshes) = resources.remove::<GpuMeshes>() else {
            counted.clear();
            return;
        };
        self.write(resources, frame, terrain, &mut meshes);
        for block in counted.drain(..) {
            self.ask(resources, block, &mut meshes);
        }
        resources.insert(meshes);
    }

    fn ask(&mut self, resources: &mut Resources, block: CountedBlock, meshes: &mut GpuMeshes) {
        self.settled = false;
        let s = self.index_of(block.key) as usize;
        self.origin[s] = block.origin;
        self.left[s] = None;
        if block.sizes.meshlets == 0 {
            self.asked[s] = None;
            self.built[s] = true;
            self.triangles[s] = 0;
            self.drop_meshes(resources, s, meshes);
            return;
        }
        self.asks = self.asks.wrapping_add(1);
        let mesh = Guid::from_bytes(slot_guid(s as u32, self.asks));
        self.asked[s] = Some(Asked { mesh, block });
        meshes.reserve(mesh, block.sizes, bounds_of(block.voxel));
    }

    fn write(
        &mut self,
        resources: &mut Resources,
        frame: &Frame,
        terrain: &mut Terrain,
        meshes: &mut GpuMeshes,
    ) {
        let granted = meshes.take_granted();
        let buffers = meshes.buffers().cloned();
        let mut jobs = Vec::with_capacity(granted.len());
        let mut done = Vec::with_capacity(granted.len());
        for (mesh, at) in granted {
            let asked = index_in(mesh)
                .and_then(|s| Some((s, self.asked.get(s).copied().flatten()?)))
                .filter(|(_, asked)| asked.mesh == mesh);
            let Some((s, Asked { block, .. })) = asked else {
                meshes.release(mesh);
                continue;
            };
            self.asked[s] = None;
            // Its slot went to another block, or its planet regenerated: the tile is not its mesh.
            let fresh = frame.pool.key(block.slot) == Some(block.key)
                && terrain.revision(block.key.planet) == block.revision;
            if !fresh || buffers.is_none() {
                meshes.release(mesh);
                continue;
            }
            // A tile it borrowed changed hands since the count: the write would mesh otherwise.
            if !block.near.holds(frame.pool, block.key) {
                meshes.release(mesh);
                terrain.retry(block.slot);
                continue;
            }
            let job = WriteJob::new(block.slot, block.voxel, block.sizes, &at, block.near);
            jobs.push(job);
            done.push((s, mesh, block));
        }
        if let Some(buffers) = &buffers {
            terrain.write(frame.device, frame.queue, buffers, &jobs);
        }
        for (s, mesh, block) in done {
            self.written(resources, frame, s, mesh, block, meshes);
        }
    }

    fn written(
        &mut self,
        resources: &mut Resources,
        frame: &Frame,
        s: usize,
        mesh: Guid,
        block: CountedBlock,
        meshes: &mut GpuMeshes,
    ) {
        let planet = frame
            .placed
            .get(block.key.planet as usize)
            .copied()
            .flatten()
            .and_then(|at| frame.found.get(at));
        let Some(planet) = planet else {
            meshes.release(mesh);
            return;
        };
        self.settled = false;
        self.built[s] = true;
        self.triangles[s] = block.sizes.triangle_bytes / 3;
        let visible = self.shown[s];
        let held = match visible {
            true => &mut self.mesh[s],
            false => &mut self.staged[s],
        };
        if let Some(old) = held.replace(mesh) {
            meshes.release(old);
        }
        let renderer = MeshRenderer {
            mesh: Some(mesh),
            material: planet.planet.material,
            visible,
            ..Default::default()
        };
        let transform = place(planet.world, block.origin);
        self.entity[s] = put(resources, self.entity[s], transform, renderer);
    }

    /// Hides the index and gives its meshes back.
    fn drop_meshes(&mut self, resources: &mut Resources, s: usize, meshes: &mut GpuMeshes) {
        for mesh in [self.mesh[s].take(), self.staged[s].take()]
            .into_iter()
            .flatten()
        {
            meshes.release(mesh);
        }
        self.shown[s] = false;
        if let Some(entity) = self.entity[s]
            && let Some(renderer) = Query::<&mut MeshRenderer>::new(resources).get(entity)
        {
            renderer.visible = false;
        }
    }

    /// Re-places the blocks of every planet that moved, turned or changed material since its blocks
    /// were drawn.
    pub(crate) fn follow(
        &mut self,
        resources: &mut Resources,
        placed: &[Option<usize>],
        found: &[Found],
    ) {
        if self.world.len() < placed.len() {
            self.world.resize(placed.len(), Mat4::NAN);
            self.material.resize(placed.len(), None);
        }
        for (index, at) in placed.iter().enumerate() {
            let Some(planet) = at.and_then(|at| found.get(at)) else {
                continue;
            };
            let material = planet.planet.material;
            if self.world[index] == planet.world && self.material[index] == material {
                continue;
            }
            self.world[index] = planet.world;
            self.material[index] = material;
            for s in 0..self.entity.len() {
                let ours = self.key[s].is_some_and(|key| key.planet == index as u32);
                let Some(entity) = self.entity[s].filter(|_| ours) else {
                    continue;
                };
                let query = Query::<(&mut Transform, &mut MeshRenderer)>::new(resources);
                if let Some((transform, renderer)) = query.get(entity) {
                    *transform = place(planet.world, self.origin[s]);
                    renderer.material = material;
                }
            }
        }
    }

    /// Triangles on screen, lingering blocks included.
    pub(crate) fn triangles(&self) -> u64 {
        let shown = self.triangles.iter().zip(&self.shown);
        shown
            .filter(|(_, on)| **on)
            .map(|(&t, _)| u64::from(t))
            .sum()
    }

    /// Whether `key`'s mesh is in hand.
    fn built(&self, key: BlockKey) -> bool {
        self.of_key
            .get(&key)
            .is_some_and(|&i| self.built[i as usize])
    }

    /// The draw index of `key`, taken from the free list when it has none.
    fn index_of(&mut self, key: BlockKey) -> u32 {
        if let Some(&i) = self.of_key.get(&key) {
            return i;
        }
        let i = self.free.pop().unwrap_or_else(|| {
            self.entity.push(None);
            self.key.push(None);
            self.origin.push(DVec3::ZERO);
            self.triangles.push(0);
            self.mesh.push(None);
            self.staged.push(None);
            self.asked.push(None);
            self.built.push(false);
            self.shown.push(false);
            self.left.push(None);
            (self.entity.len() - 1) as u32
        });
        self.key[i as usize] = Some(key);
        self.of_key.insert(key, i);
        i
    }
}

/// A block's extent in its own space: its vertices sit between one voxel before its corner and
/// its last sample.
fn bounds_of(voxel: f32) -> MeshBounds {
    let side = BLOCK_SAMPLES as f32 + 1.0;
    MeshBounds {
        center: Vec3::splat((side * 0.5 - 1.0) * voxel),
        radius: side * voxel * 3f32.sqrt() * 0.5,
    }
}

/// Updates the index's entity, or spawns one when it has none or it was despawned under us.
fn put(
    resources: &mut Resources,
    held: Option<Entity>,
    transform: Transform,
    renderer: MeshRenderer,
) -> Option<Entity> {
    if let Some(entity) = held
        && let Some((t, r)) =
            Query::<(&mut Transform, &mut MeshRenderer)>::new(resources).get(entity)
    {
        *t = transform;
        *r = renderer;
        return Some(entity);
    }
    let mut commands = resources.remove::<Commands>()?;
    let entity = commands
        .spawn(resources)
        .insert(PlanetBlock)
        .insert(transform)
        .insert(renderer)
        .id();
    resources.insert(commands);
    Some(entity)
}

/// A block's transform: its corner carried by the planet's.
fn place(world: Mat4, origin: DVec3) -> Transform {
    let (scale, rotation, _) = world.to_scale_rotation_translation();
    Transform::new(world.transform_point3(origin.as_vec3()), rotation, scale)
}

const PREFIX: &[u8; 8] = b"kooch:pl";

/// A mesh id of the draw index's own ask, in a namespace no asset's GUID uses.
fn slot_guid(index: u32, ask: u32) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(PREFIX);
    bytes[8..12].copy_from_slice(&index.to_le_bytes());
    bytes[12..].copy_from_slice(&ask.to_le_bytes());
    bytes
}

/// The draw index a [`slot_guid`] names.
fn index_in(mesh: Guid) -> Option<usize> {
    let uuid = mesh.as_uuid();
    let bytes = uuid.as_bytes();
    if bytes.get(..8)? != PREFIX {
        return None;
    }
    let index = bytes.get(8..12)?.try_into().ok()?;
    Some(u32::from_le_bytes(index) as usize)
}
