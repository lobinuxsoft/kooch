//! What draws the blocks: one mesh per draw index, handed to the renderer each frame for its
//! planet's entity, so a planet is one entity however many blocks it shows (#1991). A block that
//! changes level stays on screen until what replaces it has its mesh, as Zylann's octree does: see
//! [`swap`].
//!
//! A block's mesh is counted, asked of the meshlet pool, granted the frame after and written in
//! place by the GPU, under a GUID of its own per ask, so a stale grant never meets a newer ask.

mod swap;

use std::collections::HashMap;

use glam::{DVec3, Mat4, Vec3};
use kooch_core::Guid;
use kooch_core::resource::Resources;
use kooch_ecs::MeshRenderer;
use kooch_ecs::hierarchy::WorldOrigin;
use kooch_render::meshlet::{GpuMeshes, MeshBounds, Submitted};

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
    key: Vec<Option<BlockKey>>,
    /// The block's corner in its planet's space.
    origin: Vec<DVec3>,
    /// About how many: counted from triangle bytes, which round up per meshlet.
    triangles: Vec<u32>,
    meshlets: Vec<u32>,
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
        self.write(frame, terrain, &mut meshes);
        for block in counted.drain(..) {
            self.ask(block, &mut meshes);
        }
        resources.insert(meshes);
    }

    fn ask(&mut self, block: CountedBlock, meshes: &mut GpuMeshes) {
        self.settled = false;
        let s = self.index_of(block.key) as usize;
        self.origin[s] = block.origin;
        self.left[s] = None;
        if block.sizes.meshlets == 0 {
            self.asked[s] = None;
            self.built[s] = true;
            self.triangles[s] = 0;
            self.meshlets[s] = 0;
            self.drop_meshes(s, meshes);
            return;
        }
        self.asks = self.asks.wrapping_add(1);
        let mesh = Guid::from_bytes(slot_guid(s as u32, self.asks));
        self.asked[s] = Some(Asked { mesh, block });
        meshes.reserve(mesh, block.sizes, bounds_of(block.voxel, &block.near));
    }

    fn write(&mut self, frame: &Frame, terrain: &mut Terrain, meshes: &mut GpuMeshes) {
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
            self.written(frame, s, mesh, block, meshes);
        }
    }

    fn written(
        &mut self,
        frame: &Frame,
        s: usize,
        mesh: Guid,
        block: CountedBlock,
        meshes: &mut GpuMeshes,
    ) {
        let placed = frame.placed.get(block.key.planet as usize).copied();
        if placed
            .flatten()
            .and_then(|at| frame.found.get(at))
            .is_none()
        {
            meshes.release(mesh);
            return;
        }
        self.settled = false;
        self.built[s] = true;
        self.triangles[s] = block.sizes.triangle_bytes / 3;
        self.meshlets[s] = block.sizes.meshlets;
        let held = match self.shown[s] {
            true => &mut self.mesh[s],
            false => &mut self.staged[s],
        };
        if let Some(old) = held.replace(mesh) {
            meshes.release(old);
        }
    }

    /// Hides the index and gives its meshes back.
    fn drop_meshes(&mut self, s: usize, meshes: &mut GpuMeshes) {
        for mesh in [self.mesh[s].take(), self.staged[s].take()]
            .into_iter()
            .flatten()
        {
            meshes.release(mesh);
        }
        self.shown[s] = false;
    }

    /// Hands every block on screen to the renderer, drawn for its planet's entity. Every frame, so
    /// a planet that moved, turned or changed material, or an origin that moved, is followed at once.
    pub(crate) fn submit(
        &self,
        resources: &mut Resources,
        placed: &[Option<usize>],
        found: &[Found],
    ) {
        let origin = WorldOrigin::get(resources);
        let Some(submitted) = resources.get_mut::<Submitted>() else {
            return;
        };
        submitted.clear();
        for s in (0..self.key.len()).filter(|&s| self.shown[s]) {
            let (Some(key), Some(mesh)) = (self.key[s], self.mesh[s]) else {
                continue;
            };
            let planet = placed.get(key.planet as usize).copied().flatten();
            let Some(planet) = planet.and_then(|at| found.get(at)) else {
                continue;
            };
            let renderer = MeshRenderer {
                mesh: Some(mesh),
                material: planet.planet.material,
                ..Default::default()
            };
            submitted.push(
                planet.entity,
                place(planet, self.origin[s], &origin),
                renderer,
            );
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
            self.key.push(None);
            self.origin.push(DVec3::ZERO);
            self.triangles.push(0);
            self.meshlets.push(0);
            self.mesh.push(None);
            self.staged.push(None);
            self.asked.push(None);
            self.built.push(false);
            self.shown.push(false);
            self.left.push(None);
            (self.key.len() - 1) as u32
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

/// A block's matrix relative to the world origin: its corner carried by the planet's in f64, the
/// cells subtracted in integers, and only then narrowed. The corner is a planet radius out, where
/// f32 steps half a metre (#51).
fn place(planet: &Found, origin: DVec3, world: &WorldOrigin) -> Mat4 {
    let (scale, rotation, _) = planet.world.to_scale_rotation_translation();
    let corner = planet.at + planet.world.as_dmat4().transform_vector3(origin);
    let corner = corner - world.from(Some(&planet.cell));
    Mat4::from_scale_rotation_translation(scale, rotation, corner.as_vec3())
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
