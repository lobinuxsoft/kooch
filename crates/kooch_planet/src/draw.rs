//! The entities that draw the blocks, one per draw index. A block that changes level stays on
//! screen until what replaces it has its mesh, as Zylann's octree does, and every mesh waits until
//! the neighbours it was counted with are on screen with it: see [`swap`].
//!
//! A block's mesh is counted, asked of the meshlet pool, granted the frame after and written in
//! place by the GPU, under a GUID of its own per ask, so a stale grant never meets a newer ask.

mod agree;
mod swap;

use std::collections::HashMap;

use glam::{DVec3, Mat4, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::hierarchy::{CELL, GridCell};
use kooch_ecs::query::Query;
use kooch_ecs::{Commands, Entity, MeshRenderer, Transform};
use kooch_render::meshlet::{GpuMeshes, MeshBounds};

use crate::component::PlanetBlock;
use crate::seam::levels;
use crate::sync::Found;
use crate::{BLOCK_SAMPLES, BlockKey, BlockPool, CountedBlock, Near, Terrain, WriteJob};

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
    meshlets: Vec<u32>,
    /// The mesh on screen, and the neighbours it was counted with.
    mesh: Vec<Option<Guid>>,
    near: Vec<Near>,
    /// Written and waiting for its swap, since the frame it was written.
    staged: Vec<Option<Guid>>,
    staged_near: Vec<Near>,
    staged_at: Vec<u32>,
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
    /// Every mesh is staged for [`Blocks::swap`], an old block's too: shown at once, it would
    /// meet neighbours still drawn from before and open the seam between them (#1979).
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
            self.meshlets[s] = 0;
            self.drop_meshes(resources, s, meshes);
            return;
        }
        self.asks = self.asks.wrapping_add(1);
        let mesh = Guid::from_bytes(slot_guid(s as u32, self.asks));
        self.asked[s] = Some(Asked { mesh, block });
        meshes.reserve(mesh, block.sizes, bounds_of(block.voxel, &block.near));
    }

    fn write(
        &mut self,
        resources: &mut Resources,
        frame: &Frame,
        terrain: &mut Terrain,
        meshes: &mut GpuMeshes,
    ) {
        // A refused ask would cross a binding: its block is counted again once the budget has
        // made room, instead of waiting for a grant that never comes (#1968).
        for mesh in meshes.take_refused() {
            let Some(s) = index_in(mesh) else {
                continue;
            };
            if let Some(Some(asked)) = self.asked.get(s).copied()
                && asked.mesh == mesh
            {
                self.asked[s] = None;
                terrain.remesh(asked.block.slot);
            }
        }
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
                terrain.remesh(block.slot);
                continue;
            }
            let job = WriteJob::new(
                block.slot,
                block.voxel,
                block.sizes,
                &at,
                block.near,
                block.cuts,
            );
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
        self.meshlets[s] = block.sizes.meshlets;
        if let Some(old) = self.staged[s].replace(mesh) {
            meshes.release(old);
        }
        self.staged_near[s] = block.near;
        self.staged_at[s] = self.frame;
        let visible = self.shown[s];
        let renderer = MeshRenderer {
            mesh: if visible { self.mesh[s] } else { Some(mesh) },
            material: planet.planet.material,
            visible,
            ..Default::default()
        };
        let placed = place(planet, block.origin);
        self.entity[s] = put(resources, self.entity[s], placed, renderer);
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
                let query =
                    Query::<(&mut Transform, &mut GridCell, &mut MeshRenderer)>::new(resources);
                if let Some((transform, cell, renderer)) = query.get(entity) {
                    (*transform, *cell) = place(planet, self.origin[s]);
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

    /// Meshlets on screen, lingering blocks included.
    pub(crate) fn meshlets(&self) -> u64 {
        let shown = self.meshlets.iter().zip(&self.shown);
        shown
            .filter(|(_, on)| **on)
            .map(|(&m, _)| u64::from(m))
            .sum()
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
            self.meshlets.push(0);
            self.mesh.push(None);
            self.near.push(Near::default());
            self.staged.push(None);
            self.staged_near.push(Near::default());
            self.staged_at.push(0);
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

/// A block's extent in its own space: its vertices sit within one cell of its sides, and a cell it
/// borrows from a coarser neighbour is that neighbour's.
fn bounds_of(voxel: f32, near: &Near) -> MeshBounds {
    let up = near
        .sides
        .iter()
        .map(|&side| levels(side))
        .max()
        .unwrap_or(0);
    let reach = (1u64 << up.min(62)) as f32;
    let side = BLOCK_SAMPLES as f32;
    MeshBounds {
        center: Vec3::splat(side * 0.5 * voxel),
        radius: (side * 0.5 + reach) * voxel * 3f32.sqrt(),
    }
}

/// Updates the index's entity, or spawns one when it has none or it was despawned under us.
fn put(
    resources: &mut Resources,
    held: Option<Entity>,
    (transform, cell): (Transform, GridCell),
    renderer: MeshRenderer,
) -> Option<Entity> {
    if let Some(entity) = held
        && let Some((t, c, r)) =
            Query::<(&mut Transform, &mut GridCell, &mut MeshRenderer)>::new(resources).get(entity)
    {
        (*t, *c, *r) = (transform, cell, renderer);
        return Some(entity);
    }
    let mut commands = resources.remove::<Commands>()?;
    let entity = commands
        .spawn(resources)
        .insert(PlanetBlock)
        .insert(transform)
        .insert(cell)
        .insert(renderer)
        .id();
    resources.insert(commands);
    Some(entity)
}

/// A block's transform: its corner carried by the planet's, in f64 from the planet's centre, then
/// homed into its own cell. The corner is a planet radius out, where f32 steps half a metre (#51).
fn place(planet: &Found, origin: DVec3) -> (Transform, GridCell) {
    let (scale, rotation, _) = planet.world.to_scale_rotation_translation();
    let corner = planet.at + planet.world.as_dmat4().transform_vector3(origin);
    let whole = (corner / CELL).round();
    let by = whole.as_i64vec3() + planet.cell.cell();
    let cell = GridCell {
        x: by.x,
        y: by.y,
        z: by.z,
    };
    (Transform::new(corner - whole * CELL, rotation, scale), cell)
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
